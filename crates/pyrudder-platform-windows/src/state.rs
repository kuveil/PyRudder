//! Windows state-file I/O for drive-letter paths, with no project-owned unsafe code.
//! Windows 盘符路径的状态文件 I/O，项目自身不使用不安全代码。
//!
//! Reparse points are rejected, but checks are not handle-relative protection against hostile
//! concurrent ancestor replacement. Callers must own/trust the directories they write into.
//! 拒绝重解析点，但这些检查不是抵御恶意并发替换祖先目录的句柄相对保护。
//! 调用方必须拥有并信任要写入的目录。

use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf, Prefix},
};

use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
};

/// Native state I/O. Reads are bounded; writes publish complete last-writer-wins snapshots.
/// 原生状态 I/O。读取有大小限制；写入发布完整快照，最后成功写入者生效。
///
/// No directories are created. Atomic visibility is not a multi-file transaction or a
/// guarantee of surviving power loss; directory metadata is not explicitly flushed.
/// 不创建目录。原子可见性不是多文件事务，也不保证断电持久性；未显式刷写目录元数据。
#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsStateFileSystem;

impl StateFileSystem for WindowsStateFileSystem {
    fn normalize_directory(&self, path: &Path) -> Result<PathBuf> {
        let path = normalize_path(path)?;
        check_ancestors(&path, true)?;
        let mut existing = path.as_path();
        let mut tail = Vec::new();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(metadata) => {
                    check_type(&metadata, true)?;
                    let mut normalized = fs::canonicalize(existing)
                        .map_err(|error| io_error("normalize directory", existing, &error))?;
                    for name in tail.iter().rev() {
                        normalized.push(name);
                    }
                    return Ok(normalized);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let name = existing
                        .file_name()
                        .ok_or_else(|| io_error("locate directory root", existing, &error))?;
                    tail.push(name.to_os_string());
                    existing = existing
                        .parent()
                        .ok_or_else(|| io_error("locate parent directory", existing, &error))?;
                }
                Err(error) => return Err(io_error("normalize directory", existing, &error)),
            }
        }
    }

    fn canonical_directory(&self, path: &Path) -> Result<PathBuf> {
        let normalized = self.normalize_directory(path)?;
        let metadata = fs::symlink_metadata(&normalized)
            .map_err(|error| io_error("open directory", &normalized, &error))?;
        check_type(&metadata, true)?;
        Ok(normalized)
    }

    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        let path = normalize_path(path)?;
        check_ancestors(&path, false)?;
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error("open state file", &path, &error)),
        };
        let metadata = file
            .metadata()
            .map_err(|error| io_error("inspect state file", &path, &error))?;
        check_type(&metadata, false)?;
        let limit = u64::try_from(max_bytes)
            .ok()
            .and_then(|limit| limit.checked_add(1))
            .ok_or_else(|| Error::new(ErrorKind::Usage, "Invalid state-file read limit"))?;
        if metadata.len() >= limit {
            return Err(
                Error::new(ErrorKind::Usage, "State file exceeds its size limit").with_hint(
                    format!("Check file \"{}\"", path.to_string_lossy().escape_debug()),
                ),
            );
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error("read state file", &path, &error))?;
        if bytes.len() > max_bytes {
            return Err(Error::new(
                ErrorKind::Usage,
                "State file grew beyond its size limit",
            ));
        }
        Ok(Some(bytes))
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<()> {
        let path = normalize_path(path)?;
        let parent = path.parent().ok_or_else(|| {
            Error::new(ErrorKind::Usage, "State file requires a parent directory")
        })?;
        let parent = self.canonical_directory(parent)?;
        let name = path
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::Usage, "State file requires a file name"))?;
        let destination = parent.join(name);
        check_destination(&destination)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".pyrudder-state-")
            .suffix(".tmp")
            .tempfile_in(&parent)
            .map_err(|error| io_error("create staged state file", &parent, &error))?;
        temporary
            .write_all(contents)
            .map_err(|error| io_error("write staged state file", &destination, &error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| io_error("flush staged state file", &destination, &error))?;
        // Recheck before publishing; never remove the old file to make replacement succeed.
        // 发布前再次检查；绝不通过删除旧文件来促使替换成功。
        check_ancestors(&destination, false)?;
        check_destination(&destination)?;
        publish(temporary, &destination)
    }
}

fn publish(mut temporary: tempfile::NamedTempFile, destination: &Path) -> Result<()> {
    // Windows may report access denied during replacement while old reader handles drain.
    // Windows 在旧读取句柄释放前的替换过程中可能返回访问被拒绝。
    // Retry the same staged file with a bounded delay; permanent errors never delete the target.
    // 使用同一个暂存文件进行有界延迟重试；永久错误不会删除目标。
    let mut attempt = 0;
    loop {
        match temporary.persist(destination) {
            Ok(_) => return Ok(()),
            Err(error)
                if attempt < 7 && matches!(error.error.raw_os_error(), Some(5 | 32 | 33)) =>
            {
                temporary = error.file;
                std::thread::sleep(std::time::Duration::from_millis(1 << attempt.min(5)));
                check_ancestors(destination, false)?;
                check_destination(destination)?;
                attempt += 1;
            }
            Err(error) => return Err(io_error("publish state file", destination, &error.error)),
        }
    }
}

pub(crate) fn normalize_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::new(ErrorKind::Usage, "State paths must be absolute"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix)
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                normalized.push(prefix.as_os_str());
            }
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(name) => {
                let text = name.to_string_lossy();
                let stem = text
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(' ')
                    .to_ascii_lowercase();
                let device_number = stem
                    .strip_prefix("com")
                    .or_else(|| stem.strip_prefix("lpt"))
                    .is_some_and(|suffix| {
                        matches!(
                            suffix,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    });
                if name.encode_wide().any(|unit| {
                    unit < 32 || matches!(unit, 34 | 42 | 58 | 60 | 62 | 63 | 124 | 127)
                }) || text.ends_with(['.', ' '])
                    || matches!(
                        stem.as_str(),
                        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
                    )
                    || device_number
                {
                    return Err(Error::new(
                        ErrorKind::Usage,
                        "State path contains an invalid Windows component",
                    ));
                }
                normalized.push(name);
            }
            _ => {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Parent traversal, UNC, and device paths are not supported for state files",
                ));
            }
        }
    }
    Ok(normalized)
}

pub(crate) fn check_ancestors(path: &Path, directory: bool) -> Result<()> {
    let mut ancestors: Vec<_> = path.ancestors().collect();
    ancestors.reverse();
    for ancestor in ancestors {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => check_type(&metadata, directory || ancestor != path)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error("inspect state path", ancestor, &error)),
        }
    }
    Ok(())
}

pub(crate) fn check_type(metadata: &Metadata, directory: bool) -> Result<()> {
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error::new(
            ErrorKind::Usage,
            "State paths must not traverse reparse points",
        ));
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(Error::new(
            ErrorKind::Usage,
            "State path has an unexpected file type",
        ));
    }
    Ok(())
}

fn check_destination(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            check_type(&metadata, false)?;
            if metadata.permissions().readonly() {
                return Err(Error::new(
                    ErrorKind::Permission,
                    "State destination is read-only",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect state destination", path, &error)),
    }
}

pub(crate) fn io_error(operation: &str, path: &Path, error: &std::io::Error) -> Error {
    let kind = if matches!(error.raw_os_error(), Some(32 | 33)) {
        ErrorKind::Busy
    } else {
        match error.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::NotInstalled,
            std::io::ErrorKind::PermissionDenied => ErrorKind::Permission,
            std::io::ErrorKind::AlreadyExists => ErrorKind::Conflict,
            std::io::ErrorKind::WouldBlock => ErrorKind::Busy,
            _ => ErrorKind::Internal,
        }
    };
    Error::new(
        kind,
        format!(
            "Cannot {operation} at \"{}\" ({:?}, OS code {:?})",
            path.to_string_lossy().escape_debug(),
            error.kind(),
            error.raw_os_error()
        ),
    )
}
