//! Platform casing boundary and command identities.
//! 平台大小写边界与命令标识。

use crate::{Error, ErrorKind, Result};

/// Platform-provided non-linguistic uppercase mapping, independent of user locale.
/// 平台提供的非语言学大写映射，不依赖用户区域设置。
pub trait CommandCaseMapper {
    /// Maps a name using platform filesystem casing rules.
    /// 使用平台文件系统大小写规则映射名称。
    ///
    /// # Errors
    /// Reports native mapping failures. / 报告原生映射失败。
    fn uppercase(&self, name: &str) -> Result<String>;
}

/// Canonical name, without Unicode normalization or linguistic expansion.
/// 规范名称，不进行 Unicode 规范化或语言学展开。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandKey(String);

impl CommandKey {
    /// Validates one Windows filename component and applies platform casing.
    /// 验证单个 Windows 文件名组件并应用平台大小写规则。
    ///
    /// # Errors
    /// Rejects paths, controls, device names, and names over 255 UTF-16 units.
    /// 拒绝路径、控制字符、设备名以及超过 255 个 UTF-16 单元的名称。
    pub fn new(name: &str, mapper: &impl CommandCaseMapper) -> Result<Self> {
        validate_name(name)?;
        let canonical = mapper.uppercase(name)?;
        validate_name(&canonical)?;
        Ok(Self(canonical))
    }

    /// Mapped name, not original display spelling. / 映射后的名称，而非原始显示拼写。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_name(name: &str) -> Result<()> {
    let first = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    let device = first.to_ascii_lowercase();
    let numbered_device = device
        .strip_prefix("com")
        .or_else(|| device.strip_prefix("lpt"))
        .is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    if name.is_empty()
        || name.encode_utf16().take(256).count() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
        || matches!(
            device.as_str(),
            "con" | "prn" | "aux" | "nul" | "conin$" | "conout$"
        )
        || numbered_device
    {
        return Err(Error::new(ErrorKind::Usage, "Invalid Windows command name"));
    }
    Ok(())
}

/// Supported extensions in fixed precedence order; PATHEXT is deliberately not read.
/// 按固定优先级排列的受支持扩展；特意不读取 PATHEXT。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CommandExtension {
    /// Native executable. / 原生可执行文件。
    Exe,
    /// COM-named native executable. / 使用 COM 扩展名的原生可执行文件。
    Com,
    /// Command Prompt script. / 命令提示符脚本。
    Cmd,
    /// Batch script. / 批处理脚本。
    Bat,
    /// `PowerShell` script. / `PowerShell` 脚本。
    PowerShell,
}

impl CommandExtension {
    /// Splits the final supported extension without changing the original stem.
    /// 拆分末尾受支持扩展，不改变原始主名称。
    #[must_use]
    pub fn split(name: &str) -> Option<(&str, Self)> {
        let (stem, extension) = name.rsplit_once('.')?;
        let extension = match extension.to_ascii_lowercase().as_str() {
            "exe" => Self::Exe,
            "com" => Self::Com,
            "cmd" => Self::Cmd,
            "bat" => Self::Bat,
            "ps1" => Self::PowerShell,
            _ => return None,
        };
        Some((stem, extension))
    }
}

/// Bare logical names and explicit filenames have separate namespaces.
/// 无扩展逻辑名和显式文件名拥有独立命名空间。
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CommandRequest {
    /// Logical stem, e.g. `pip`. / 逻辑主名称，例如 `pip`。
    Bare(CommandKey),
    /// Full filename, e.g. `pip.cmd`. / 完整文件名，例如 `pip.cmd`。
    Exact(CommandKey),
}

/// Protects the manager namespace and avoids taking over the official Python launcher.
/// 保护管理器命名空间，避免接管官方 Python 启动器。
#[must_use]
pub fn is_reserved(stem: &str) -> bool {
    let stem = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    stem == "py" || stem == "pyrudder" || stem.starts_with("pyrudder-")
}
