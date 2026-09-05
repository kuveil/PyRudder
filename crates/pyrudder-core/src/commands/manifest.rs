//! Deterministic command snapshots and strict per-runtime lookup.
//! 确定性命令快照及严格的单运行时查询。

use super::{
    CommandCaseMapper, CommandExtension, CommandKey, CommandKind, CommandRequest, is_reserved,
};
use crate::{Error, ErrorKind, Result, runtime::RuntimeId};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Component, PathBuf},
    time::SystemTime,
};

/// Source priority: root, then recorded script-directory order.
/// 来源优先级：根目录，然后按记录顺序排列的脚本目录。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CommandOrigin {
    /// Runtime root. / 运行时根目录。
    Root,
    /// Zero-based script-directory index. / 从零开始的脚本目录索引。
    Script(usize),
}

/// Rejection retained instead of silently trying a lower-priority file.
/// 保留的拒绝原因，不静默尝试低优先级文件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandRejection {
    /// Malformed or truncated PE header. / PE 文件头损坏或被截断。
    InvalidPe,
    /// Unsupported architecture, subsystem, DLL, or legacy DOS COM file.
    /// 不支持的架构、子系统、DLL 或传统 DOS COM 文件。
    UnsupportedImage,
    /// Symlink, junction, or another reparse point. / 符号链接、junction 或其他重解析点。
    ReparsePoint,
}

/// Header classification, not proof that Windows can load or safely run the file.
/// 文件头分类，不保证 Windows 可以加载或安全执行该文件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandStatus {
    /// Supported candidate. / 受支持的候选项。
    Available(CommandKind),
    /// Rejected candidate, still masking lower-priority candidates.
    /// 被拒绝的候选项，仍遮蔽低优先级候选项。
    Rejected(CommandRejection),
}

/// Metadata observation, not file identity or integrity proof.
/// 元数据观测，不是文件身份或完整性证明。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryFingerprint {
    /// Original filename, including unsupported names. / 原始文件名，包括不受支持的名称。
    pub name: OsString,
    /// Observed size. / 观测到的大小。
    pub size: u64,
    /// Observed last-write time. / 观测到的最后写入时间。
    pub modified: SystemTime,
    /// Native attributes, including directory and reparse flags. / 原生属性，包括目录和重解析标志。
    pub attributes: u32,
}

/// Exact sorted metadata snapshot; not a persisted format or a content hash.
/// 精确排序的元数据快照；不是持久化格式或内容哈希。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryFingerprint {
    /// Canonical/normalized absolute directory. / 规范化的绝对目录。
    pub path: PathBuf,
    /// None means a missing optional script directory. / None 表示可选脚本目录不存在。
    pub modified: Option<SystemTime>,
    /// Direct children in original filename order. / 按原始文件名排序的直接子项。
    pub entries: Vec<EntryFingerprint>,
}

/// A discovered file; dispatch must revalidate its identity, boundary, and availability.
/// 发现的文件；分派时必须重新验证其身份、边界和可用性。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandTarget {
    /// Absolute source path. / 来源绝对路径。
    pub path: PathBuf,
    /// Ordered source directory. / 有序来源目录。
    pub origin: CommandOrigin,
    /// Actual filename extension. / 实际文件扩展名。
    pub extension: CommandExtension,
    /// File classification or rejection. / 文件分类或拒绝原因。
    pub status: CommandStatus,
    /// Metadata captured during scanning. / 扫描时捕获的元数据。
    pub fingerprint: EntryFingerprint,
}

/// Winning target and all shadowed candidates, in deterministic priority order.
/// 获选目标及所有被遮蔽候选项，按确定性优先级排序。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEntry {
    /// First candidate, even if rejected. / 首选候选项，即使它被拒绝。
    pub winner: CommandTarget,
    /// Other candidates; never implicit fallbacks. / 其他候选项；绝不作为隐式回退。
    pub shadowed: Vec<CommandTarget>,
}

/// A non-fatal discovery diagnostic. / 非致命的命令发现诊断。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryDiagnostic {
    /// Affected path. / 相关路径。
    pub path: PathBuf,
    /// Diagnostic classification. / 诊断分类。
    pub reason: DiscoveryIssue,
}

/// Reasons that do not discard the entire inventory. / 不导致整份清单被丢弃的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryIssue {
    /// Missing optional scripts directory. / 可选脚本目录不存在。
    MissingDirectory,
    /// Directory already scanned through an equivalent path. / 等价路径目录已扫描。
    DuplicateDirectory,
    /// Reserved manager namespace or official `py` launcher. / 保留的管理器命名空间或官方 `py` 启动器。
    ReservedName,
    /// Invalid Windows name or non-Unicode filename. / 无效 Windows 名称或非 Unicode 文件名。
    InvalidName,
    /// Rejected command candidate. / 被拒绝的命令候选项。
    Rejected(CommandRejection),
}

/// In-memory inventory of one runtime, without execution or publication.
/// 单个运行时的内存清单，不执行进程或发布文件。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeCommandManifest {
    runtime_id: RuntimeId,
    commands: BTreeMap<CommandRequest, CommandEntry>,
    directories: Vec<DirectoryFingerprint>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

impl RuntimeCommandManifest {
    /// Builds an inventory using source order, extension order, then original UTF-16 spelling.
    /// 使用来源顺序、扩展顺序和原始 UTF-16 拼写构建清单。
    ///
    /// # Errors
    /// Rejects malformed/inconsistent targets, duplicate observations, or failed native casing.
    /// 拒绝无效/不一致的目标、重复观测或失败的原生大小写映射。
    pub fn build(
        runtime_id: RuntimeId,
        mut targets: Vec<CommandTarget>,
        directories: Vec<DirectoryFingerprint>,
        diagnostics: Vec<DiscoveryDiagnostic>,
        mapper: &impl CommandCaseMapper,
    ) -> Result<Self> {
        targets.sort_by(|left, right| {
            (left.origin, left.extension)
                .cmp(&(right.origin, right.extension))
                .then_with(|| {
                    let left = left
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    let right = right
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    left.encode_utf16().cmp(right.encode_utf16())
                })
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut commands: BTreeMap<CommandRequest, CommandEntry> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for target in targets {
            if !seen.insert((target.origin, target.path.clone())) {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "Duplicate command target observation",
                ));
            }
            if !target.path.is_absolute()
                || target
                    .path
                    .components()
                    .any(|part| part == Component::ParentDir)
            {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Command targets must be absolute",
                ));
            }
            let name = target
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| Error::new(ErrorKind::Usage, "Invalid command filename"))?;
            let (stem, extension) = CommandExtension::split(name)
                .ok_or_else(|| Error::new(ErrorKind::Usage, "Unsupported command extension"))?;
            if extension != target.extension
                || is_reserved(stem)
                || target.fingerprint.name.to_str() != Some(name)
                || !classification_matches(extension, target.status)
            {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Reserved or inconsistent command target",
                ));
            }
            let requests = [
                CommandRequest::Bare(CommandKey::new(stem, mapper)?),
                CommandRequest::Exact(CommandKey::new(name, mapper)?),
            ];
            for request in requests {
                commands
                    .entry(request)
                    .and_modify(|entry| entry.shadowed.push(target.clone()))
                    .or_insert_with(|| CommandEntry {
                        winner: target.clone(),
                        shadowed: Vec::new(),
                    });
            }
        }
        Ok(Self {
            runtime_id,
            commands,
            directories,
            diagnostics,
        })
    }

    /// Exact runtime identity. / 精确运行时标识。
    #[must_use]
    pub const fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    /// Requests and their conflict records. / 请求及对应冲突记录。
    #[must_use]
    pub const fn commands(&self) -> &BTreeMap<CommandRequest, CommandEntry> {
        &self.commands
    }

    /// Ordered directory observations. / 有序目录观测。
    #[must_use]
    pub fn directories(&self) -> &[DirectoryFingerprint] {
        &self.directories
    }

    /// Non-fatal discovery diagnostics. / 非致命的发现诊断。
    #[must_use]
    pub fn diagnostics(&self) -> &[DiscoveryDiagnostic] {
        &self.diagnostics
    }

    /// Looks up only this runtime, without checking or executing the target file.
    /// 只查询当前运行时，不检查或执行目标文件。
    ///
    /// # Errors
    /// Missing commands return 11; rejected winners return 12, without fallback.
    /// 命令缺失返回 11；获选项被拒绝返回 12，不回退。
    pub fn lookup(&self, request: &CommandRequest) -> Result<&CommandTarget> {
        let entry = self.commands.get(request).ok_or_else(|| {
            Error::new(
                ErrorKind::CommandMissing,
                "Command is missing from the selected runtime",
            )
        })?;
        if matches!(entry.winner.status, CommandStatus::Rejected(_)) {
            return Err(Error::new(
                ErrorKind::BrokenRuntime,
                "Selected command candidate was rejected",
            )
            .with_hint(format!(
                "Inspect \"{}\" and refresh the manifest",
                entry.winner.path.to_string_lossy().escape_debug()
            )));
        }
        Ok(&entry.winner)
    }
}

fn classification_matches(extension: CommandExtension, status: CommandStatus) -> bool {
    match status {
        CommandStatus::Rejected(_) => true,
        CommandStatus::Available(kind) => matches!(
            (extension, kind),
            (
                CommandExtension::Exe | CommandExtension::Com,
                CommandKind::PeConsole | CommandKind::PeGui
            ) | (CommandExtension::Cmd, CommandKind::Cmd)
                | (CommandExtension::Bat, CommandKind::Bat)
                | (CommandExtension::PowerShell, CommandKind::PowerShell)
        ),
    }
}

/// Union for future shim publication; values are diagnostic providers, not fallbacks.
/// 供后续 shim 发布使用的并集；值是诊断用提供者，不是回退目标。
pub type CommandUnion = BTreeMap<CommandRequest, BTreeSet<RuntimeId>>;

/// Computes a deterministic union of supported winning entries.
/// 计算受支持获选项的确定性并集。
///
/// # Errors
/// Rejects ambiguous duplicate runtime manifests. / 拒绝有歧义的重复运行时清单。
pub fn command_union(manifests: &[RuntimeCommandManifest]) -> Result<CommandUnion> {
    let mut seen = BTreeSet::new();
    let mut union: CommandUnion = BTreeMap::new();
    for manifest in manifests {
        if !seen.insert(manifest.runtime_id.clone()) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "Duplicate runtime command manifests",
            ));
        }
        for (request, entry) in &manifest.commands {
            if matches!(entry.winner.status, CommandStatus::Available(_)) {
                union
                    .entry(request.clone())
                    .or_default()
                    .insert(manifest.runtime_id.clone());
            }
        }
    }
    Ok(union)
}

/// Looks up an exact runtime ID; providers in other manifests are never tried.
/// 按精确运行时 ID 查询；绝不尝试其他清单中的提供者。
///
/// # Errors
/// Reports missing/duplicate manifests, absent commands, and rejected winners.
/// 报告清单缺失/重复、命令缺失以及获选项被拒绝。
pub fn lookup_command<'a>(
    manifests: &'a [RuntimeCommandManifest],
    active: &RuntimeId,
    request: &CommandRequest,
) -> Result<&'a CommandTarget> {
    let mut matches = manifests
        .iter()
        .filter(|manifest| manifest.runtime_id() == active);
    let manifest = matches.next().ok_or_else(|| {
        Error::new(
            ErrorKind::NotInstalled,
            "Selected runtime has no command manifest",
        )
    })?;
    if matches.next().is_some() {
        return Err(Error::new(
            ErrorKind::Conflict,
            "Selected runtime has duplicate manifests",
        ));
    }
    manifest.lookup(request)
}
