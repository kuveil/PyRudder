//! Bounded selection files and an injectable platform I/O boundary.
//! 有大小限制的选择文件，以及可注入的平台 I/O 边界。

use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    Error, ErrorKind, Result,
    runtime::{RuntimeId, RuntimeRecord},
    selector::{RuntimeSelection, VersionSelector},
};

/// Maximum selection-file bytes, including an optional BOM and CRLF.
/// 选择文件最大字节数，包括可选的 BOM 与 CRLF。
pub const MAX_SELECTION_FILE_BYTES: usize = 133;

/// Platform operations used by configuration, state, and resolution.
/// 配置、状态与解析使用的平台操作。
pub trait StateFileSystem {
    /// Reads one ordinary file; absence alone returns `None`. Rejects reparse points and oversized data.
    /// 读取单个普通文件；仅缺失返回 `None`，拒绝重解析点和超限数据。
    ///
    /// # Errors
    /// Returns path, type, size, or I/O failures, never silently treating them as absence.
    /// 返回路径、类型、大小或 I/O 错误，不将这些错误静默视为缺失。
    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>>;

    /// Normalizes an absolute directory, including a possibly missing tail, without creating it.
    /// 规范化绝对目录（允许尾部尚不存在），不创建目录。
    ///
    /// # Errors
    /// Rejects unsafe or unsupported paths and inaccessible existing ancestors.
    /// 拒绝不安全或不支持的路径，以及不可访问的已有祖先目录。
    fn normalize_directory(&self, path: &Path) -> Result<PathBuf>;

    /// Returns the canonical identity path of an existing directory for boundary comparisons.
    /// 返回已有目录的规范身份路径，用于边界比较。
    ///
    /// # Errors
    /// Rejects missing, inaccessible, or unsupported directories.
    /// 拒绝缺失、不可访问或不支持的目录。
    fn canonical_directory(&self, path: &Path) -> Result<PathBuf>;

    /// Atomically replaces a file in an existing, caller-owned directory; no read-modify-write.
    /// 在已有、调用方拥有的目录中原子替换文件；不提供读改写事务。
    ///
    /// # Errors
    /// Returns validation or I/O failures; failures before publication preserve the old file.
    /// 返回校验或 I/O 错误；发布前失败须保留旧文件。
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<()>;
}

/// Parses UTF-8 single-line text, accepting one optional BOM and one LF or CRLF terminator.
/// 解析 UTF-8 单行文本，允许一个可选 BOM 和一个 LF 或 CRLF 结尾。
///
/// # Errors
/// Rejects excessive length, invalid encoding, extra lines, whitespace, or invalid selectors.
/// 拒绝超长、编码错误、多行、空白或非法选择器。
pub fn parse_selection_file(bytes: &[u8]) -> Result<VersionSelector> {
    if bytes.len() > MAX_SELECTION_FILE_BYTES {
        return Err(Error::new(
            ErrorKind::Usage,
            "Selection file exceeds 133 bytes",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::new(ErrorKind::Usage, "Selection file must be UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    text.parse()
}

/// Reads and validates a selection file without interpreting it as a command or path.
/// 读取并校验选择文件，不将其解释为命令或路径。
///
/// # Errors
/// Propagates file or selector errors with an escaped source-path hint.
/// 传递文件或选择器错误，并附带经过转义的来源路径提示。
pub fn read_selection_file(
    fs: &impl StateFileSystem,
    path: &Path,
) -> Result<Option<VersionSelector>> {
    fs.read_file(path, MAX_SELECTION_FILE_BYTES)?
        .map(|bytes| {
            parse_selection_file(&bytes).map_err(|error| {
                error.with_hint(format!(
                    "Check selection file \"{}\"",
                    path.to_string_lossy().escape_debug()
                ))
            })
        })
        .transpose()
}

/// A stable on-disk choice, never a floating version or alias.
/// 稳定的落盘选择，不包含浮动版本或别名。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PinnedSelection {
    /// Exact installation identity. / 精确安装标识。
    Runtime(RuntimeId),
    /// Explicit system request, still requiring dispatch-time PATH filtering.
    /// 显式系统请求，分派时仍需过滤 PATH。
    System,
}

impl fmt::Display for PinnedSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(id) => id.fmt(formatter),
            Self::System => formatter.write_str("system"),
        }
    }
}

/// Resolves a choice before atomically publishing its exact ID plus LF.
/// 先解析选择，再原子发布精确 ID 及 LF 结尾。
///
/// The caller supplies an authorized local/global target with an existing parent directory.
/// 调用方提供已授权的 local/global 目标，且其父目录必须已存在。
///
/// # Errors
/// Invalid, missing, ambiguous, unhealthy, or policy-disallowed selections never write a file.
/// 无效、缺失、有歧义、不健康或被策略禁止的选择不会写入文件。
pub fn write_selection_file(
    fs: &impl StateFileSystem,
    path: &Path,
    selector: &VersionSelector,
    runtimes: &[RuntimeRecord],
    allow_system: bool,
) -> Result<PinnedSelection> {
    let pinned = match selector.select(runtimes)? {
        RuntimeSelection::Registered(runtime) => PinnedSelection::Runtime(runtime.id().clone()),
        RuntimeSelection::SystemRequested => {
            check_system_policy(allow_system)?;
            PinnedSelection::System
        }
    };
    fs.write_atomic(path, format!("{pinned}\n").as_bytes())?;
    Ok(pinned)
}

pub(crate) fn check_system_policy(allowed: bool) -> Result<()> {
    if !allowed {
        return Err(
            Error::new(ErrorKind::Usage, "System fallback is disabled").with_hint(
                "Explicitly enable commands.system_fallback in trusted user configuration",
            ),
        );
    }
    Ok(())
}
