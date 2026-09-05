//! Stable management errors; child-process exit codes are forwarded separately.
//! 稳定的管理错误；子进程退出码另行原样转发。

use std::fmt;

/// Stable management failure categories; success is exit code zero.
/// 稳定的管理失败类别；成功的退出码为零。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ErrorKind {
    /// Invalid arguments or selector syntax. / 参数或选择器语法无效。
    Usage = 2,
    /// No registered runtime matches. / 没有匹配的已登记运行时。
    NotInstalled = 10,
    /// The active runtime lacks a command. / 活动运行时缺少命令。
    CommandMissing = 11,
    /// The selected runtime is unhealthy. / 所选运行时健康状态异常。
    BrokenRuntime = 12,
    /// Ambiguous selection or conflicting state. / 选择存在歧义或状态冲突。
    Conflict = 13,
    /// Network operation failed. / 网络操作失败。
    Network = 20,
    /// Digest or signature verification failed. / 摘要或签名校验失败。
    Integrity = 21,
    /// Installation or its health check failed. / 安装或安装健康检查失败。
    Install = 22,
    /// Access was denied. / 访问被拒绝。
    Permission = 30,
    /// A resource is in use. / 资源正在使用中。
    Busy = 31,
    /// Current-shell switching requires a hook. / 当前终端切换需要 hook。
    ShellHookRequired = 40,
    /// An unexpected internal failure occurred. / 出现未预期的内部错误。
    Internal = 70,
}

impl ErrorKind {
    /// Returns the stable management exit code. / 返回稳定的管理退出码。
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        self as u8
    }

    /// Returns a stable key for future structured output. / 返回供结构化输出使用的稳定键。
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::NotInstalled => "not_installed",
            Self::CommandMissing => "command_missing",
            Self::BrokenRuntime => "broken_runtime",
            Self::Conflict => "conflict",
            Self::Network => "network",
            Self::Integrity => "integrity",
            Self::Install => "install",
            Self::Permission => "permission",
            Self::Busy => "busy",
            Self::ShellHookRequired => "shell_hook_required",
            Self::Internal => "internal",
        }
    }
}

/// A management failure with a stable category, message, and optional remedy.
/// 包含稳定类别、消息和可选修复建议的管理错误。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    hint: Option<String>,
}

impl Error {
    /// Creates an error; callers must redact secrets in messages.
    /// 创建错误；调用方必须隐去消息中的秘密信息。
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
        }
    }

    /// Adds actionable repair guidance. / 添加可执行的修复建议。
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Returns the error category. / 返回错误类别。
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the management exit code. / 返回管理退出码。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        self.kind.exit_code()
    }

    /// Returns the explanation without the remedy. / 返回不含修复建议的解释。
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns optional repair guidance. / 返回可选的修复建议。
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind.code(), self.message)?;
        if let Some(hint) = &self.hint {
            write!(formatter, "\nHint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// Result type shared by domain operations. / 领域操作共享的结果类型。
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid_input(message: &'static str) -> Error {
    // Never echo untrusted selector text (including terminal escape sequences).
    // 不回显不可信选择器文本（包括终端转义序列）。
    Error::new(ErrorKind::Usage, message)
}
