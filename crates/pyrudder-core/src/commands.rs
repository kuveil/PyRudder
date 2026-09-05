//! Read-only command inventories; snapshots never authorize execution.
//! 只读命令清单；快照不构成执行授权。

mod manifest;
mod names;

pub use manifest::*;
pub use names::*;

/// A command target supported by the Windows command router.
/// Windows 命令路由器支持的命令目标类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    /// A PE executable using the console subsystem.
    /// 使用控制台子系统的 PE 可执行文件。
    PeConsole,
    /// A PE executable using the Windows GUI subsystem.
    /// 使用 Windows GUI 子系统的 PE 可执行文件。
    PeGui,
    /// A Command Prompt `.cmd` script.
    /// 命令提示符 `.cmd` 脚本。
    Cmd,
    /// A Command Prompt `.bat` script.
    /// 命令提示符 `.bat` 脚本。
    Bat,
    /// A `PowerShell` `.ps1` script.
    /// `PowerShell` `.ps1` 脚本。
    PowerShell,
}

impl CommandKind {
    /// Returns whether this command is a native PE executable.
    /// 返回该命令是否为原生 PE 可执行文件。
    #[must_use]
    pub const fn is_pe(self) -> bool {
        matches!(self, Self::PeConsole | Self::PeGui)
    }
}

#[cfg(test)]
mod tests {
    use super::CommandKind;

    #[test]
    fn identifies_pe_commands() {
        assert!(CommandKind::PeConsole.is_pe());
        assert!(CommandKind::PeGui.is_pe());
        assert!(!CommandKind::Cmd.is_pe());
    }
}
