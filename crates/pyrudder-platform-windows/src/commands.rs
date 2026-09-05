//! Read-only Windows command discovery; never executes files or modifies PATH.
//! 只读 Windows 命令发现；绝不执行文件或修改 PATH。
//!
//! Directory checks reject reparse points but are not handle-anchored protection against
//! hostile concurrent ancestor replacement. Dispatch must revalidate every target.
//! 目录检查拒绝重解析点，但不是抵御恶意并发替换祖先目录的句柄锚定防护。
//! 分派时必须重新验证每个目标。

use crate::{
    casing, pe,
    state::{WindowsStateFileSystem, check_ancestors, check_type, io_error, normalize_path},
};
use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{
        CommandCaseMapper, CommandExtension, CommandKey, CommandKind, CommandOrigin,
        CommandRejection, CommandStatus, CommandTarget, DirectoryFingerprint, DiscoveryDiagnostic,
        DiscoveryIssue, EntryFingerprint, RuntimeCommandManifest, is_reserved,
    },
    runtime::RuntimeRecord,
    state::StateFileSystem,
};
use std::{
    collections::BTreeSet,
    fs::{self, Metadata, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

/// Invariant Windows filesystem uppercase mapping, without linguistic casing flags.
/// 固定区域的 Windows 文件系统大写映射，不启用语言学大小写标志。
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsCommandCaseMapper;

impl CommandCaseMapper for WindowsCommandCaseMapper {
    fn uppercase(&self, name: &str) -> Result<String> {
        casing::uppercase(name)
    }
}

/// Resource bounds for a single scan, including unsupported directory entries.
/// 单次扫描的资源边界，包含不受支持的目录项。
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryLimits {
    /// Root plus declared scripts directories, at most 64. / 根目录及声明的脚本目录，最多 64 个。
    pub directories: usize,
    /// Total direct children, at most 16,384. / 直接子项总数，最多 16,384 个。
    pub entries: usize,
}

impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            directories: 64,
            entries: 16_384,
        }
    }
}

/// Scans only root and explicitly registered scripts directories, without probing Python.
/// 只扫描根目录及显式登记的脚本目录，不探测 Python。
///
/// Missing script directories are recorded; a missing root or any I/O failure aborts.
/// 记录缺失的脚本目录；根目录缺失或任意 I/O 失败会中止扫描。
///
/// # Errors
/// Reports invalid paths, I/O failures, detected changes, or resource-limit violations.
/// 报告无效路径、I/O 失败、检测到的变更或资源超限。
pub fn discover_commands(runtime: &RuntimeRecord) -> Result<RuntimeCommandManifest> {
    discover_commands_with_limits(runtime, DiscoveryLimits::default())
}

/// Performs the same read-only scan with smaller configurable resource bounds.
/// 使用可配置的较小资源边界进行同样的只读扫描。
///
/// # Errors
/// Also rejects zero limits or limits exceeding the built-in maximums.
/// 还会拒绝零限制或超过内置上限的限制。
pub fn discover_commands_with_limits(
    runtime: &RuntimeRecord,
    limits: DiscoveryLimits,
) -> Result<RuntimeCommandManifest> {
    if limits.directories == 0
        || limits.directories > 64
        || limits.entries == 0
        || limits.entries > 16_384
        || runtime.command_dirs().len() >= limits.directories
    {
        return Err(Error::new(
            ErrorKind::Usage,
            "Command discovery directory/entry limits exceeded",
        ));
    }
    let mut scan = Scan {
        limits,
        visited_entries: 0,
        targets: Vec::new(),
        directories: Vec::new(),
        diagnostics: Vec::new(),
    };
    let sources = std::iter::once((CommandOrigin::Root, runtime.root())).chain(
        runtime
            .command_dirs()
            .iter()
            .enumerate()
            .map(|(index, path)| (CommandOrigin::Script(index), path.as_path())),
    );
    let mut seen = BTreeSet::new();
    for (origin, source) in sources {
        let path = WindowsStateFileSystem.normalize_directory(source)?;
        let identity = path
            .to_str()
            .ok_or_else(|| Error::new(ErrorKind::Usage, "Command directory must be Unicode"))?;
        if !seen.insert(casing::uppercase(identity)?) {
            scan.diagnostic(path, DiscoveryIssue::DuplicateDirectory);
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && origin != CommandOrigin::Root =>
            {
                scan.diagnostic(path.clone(), DiscoveryIssue::MissingDirectory);
                scan.directories.push(DirectoryFingerprint {
                    path,
                    modified: None,
                    entries: Vec::new(),
                });
                continue;
            }
            Err(error) => return Err(io_error("inspect command directory", &path, &error)),
        };
        check_type(&metadata, true)?;
        scan.directory(path, origin, &metadata)?;
    }
    RuntimeCommandManifest::build(
        runtime.id().clone(),
        scan.targets,
        scan.directories,
        scan.diagnostics,
        &WindowsCommandCaseMapper,
    )
}

struct Scan {
    limits: DiscoveryLimits,
    visited_entries: usize,
    targets: Vec<CommandTarget>,
    directories: Vec<DirectoryFingerprint>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

impl Scan {
    fn diagnostic(&mut self, path: PathBuf, reason: DiscoveryIssue) {
        self.diagnostics.push(DiscoveryDiagnostic { path, reason });
    }

    fn directory(&mut self, path: PathBuf, origin: CommandOrigin, before: &Metadata) -> Result<()> {
        let modified = before
            .modified()
            .map_err(|error| io_error("read directory timestamp", &path, &error))?;
        let mut children = Vec::new();
        for entry in
            fs::read_dir(&path).map_err(|error| io_error("enumerate commands", &path, &error))?
        {
            let entry = entry.map_err(|error| io_error("read directory entry", &path, &error))?;
            self.visited_entries += 1;
            if self.visited_entries > self.limits.entries {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Command discovery entry limit exceeded",
                ));
            }
            children.push(entry.path());
        }
        children.sort_by(|left, right| {
            left.as_os_str()
                .encode_wide()
                .cmp(right.as_os_str().encode_wide())
        });
        let mut entries = Vec::with_capacity(children.len());
        for child in children {
            let metadata = fs::symlink_metadata(&child)
                .map_err(|error| io_error("inspect command entry", &child, &error))?;
            let observed = fingerprint(&child, &metadata)?;
            self.entry(&child, origin, &metadata, &observed)?;
            entries.push(observed);
        }
        check_ancestors(&path, true)?;
        let after = fs::symlink_metadata(&path)
            .map_err(|error| io_error("recheck command directory", &path, &error))?;
        if before.last_write_time() != after.last_write_time()
            || before.creation_time() != after.creation_time()
        {
            return Err(changed(&path));
        }
        self.directories.push(DirectoryFingerprint {
            path,
            modified: Some(modified),
            entries,
        });
        Ok(())
    }

    fn entry(
        &mut self,
        path: &Path,
        origin: CommandOrigin,
        metadata: &Metadata,
        observed: &EntryFingerprint,
    ) -> Result<()> {
        let reparse = metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
        if metadata.is_dir() && !reparse {
            return Ok(());
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            self.diagnostic(path.to_path_buf(), DiscoveryIssue::InvalidName);
            return Ok(());
        };
        let Some((stem, extension)) = CommandExtension::split(name) else {
            return Ok(());
        };
        if is_reserved(stem) {
            self.diagnostic(path.to_path_buf(), DiscoveryIssue::ReservedName);
            return Ok(());
        }
        for name in [stem, name] {
            if let Err(error) = CommandKey::new(name, &WindowsCommandCaseMapper) {
                if error.kind() != ErrorKind::Usage {
                    return Err(error);
                }
                self.diagnostic(path.to_path_buf(), DiscoveryIssue::InvalidName);
                return Ok(());
            }
        }
        let status = if reparse {
            CommandStatus::Rejected(CommandRejection::ReparsePoint)
        } else {
            classify_file(path, extension, observed)?
        };
        if let CommandStatus::Rejected(reason) = status {
            self.diagnostic(path.to_path_buf(), DiscoveryIssue::Rejected(reason));
        }
        self.targets.push(CommandTarget {
            path: path.to_path_buf(),
            origin,
            extension,
            status,
            fingerprint: observed.clone(),
        });
        Ok(())
    }
}

fn classify_file(
    path: &Path,
    extension: CommandExtension,
    observed: &EntryFingerprint,
) -> Result<CommandStatus> {
    let path = normalize_path(path)?;
    check_ancestors(&path, false)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| io_error("open command header", &path, &error))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error("inspect command handle", &path, &error))?;
    check_type(&metadata, false)?;
    if &fingerprint(&path, &metadata)? != observed {
        return Err(changed(&path));
    }
    let kind = match extension {
        CommandExtension::Exe | CommandExtension::Com => pe::classify(&mut file, metadata.len())
            .map_err(|error| io_error("read PE header", &path, &error))?,
        CommandExtension::Cmd => Ok(CommandKind::Cmd),
        CommandExtension::Bat => Ok(CommandKind::Bat),
        CommandExtension::PowerShell => Ok(CommandKind::PowerShell),
    };
    let after = file
        .metadata()
        .map_err(|error| io_error("recheck command handle", &path, &error))?;
    if fingerprint(&path, &after)? != *observed {
        return Err(changed(&path));
    }
    Ok(match kind {
        Ok(kind) => CommandStatus::Available(kind),
        Err(CommandRejection::InvalidPe) if extension == CommandExtension::Com => {
            CommandStatus::Rejected(CommandRejection::UnsupportedImage)
        }
        Err(reason) => CommandStatus::Rejected(reason),
    })
}

fn fingerprint(path: &Path, metadata: &Metadata) -> Result<EntryFingerprint> {
    Ok(EntryFingerprint {
        name: path
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::Usage, "Command entry requires a name"))?
            .to_os_string(),
        size: metadata.len(),
        modified: metadata
            .modified()
            .map_err(|error| io_error("read command timestamp", path, &error))?,
        attributes: metadata.file_attributes(),
    })
}

fn changed(path: &Path) -> Error {
    Error::new(
        ErrorKind::Busy,
        "Command directory or file changed during discovery",
    )
    .with_hint(format!(
        "Retry discovery of \"{}\"",
        path.to_string_lossy().escape_debug()
    ))
}
