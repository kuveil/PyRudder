//! Stable public management error contract.
//! 稳定的公开管理错误契约。

use pyrudder_core::{Error, ErrorKind};

#[test]
fn management_exit_codes_and_structured_keys_remain_stable() {
    for (kind, exit_code, code) in [
        (ErrorKind::Usage, 2, "usage"),
        (ErrorKind::NotInstalled, 10, "not_installed"),
        (ErrorKind::CommandMissing, 11, "command_missing"),
        (ErrorKind::BrokenRuntime, 12, "broken_runtime"),
        (ErrorKind::Conflict, 13, "conflict"),
        (ErrorKind::Network, 20, "network"),
        (ErrorKind::Integrity, 21, "integrity"),
        (ErrorKind::Install, 22, "install"),
        (ErrorKind::Permission, 30, "permission"),
        (ErrorKind::Busy, 31, "busy"),
        (ErrorKind::ShellHookRequired, 40, "shell_hook_required"),
        (ErrorKind::Internal, 70, "internal"),
    ] {
        assert_eq!(kind.exit_code(), exit_code);
        assert_eq!(kind.code(), code);
        assert_eq!(Error::new(kind, "message").exit_code(), exit_code);
    }
}

#[test]
fn management_errors_include_optional_repair_guidance() {
    let error = Error::new(ErrorKind::NotInstalled, "No matching runtime");
    assert_eq!(error.kind(), ErrorKind::NotInstalled);
    assert_eq!(error.message(), "No matching runtime");
    assert_eq!(error.hint(), None);
    assert_eq!(error.to_string(), "not_installed: No matching runtime");
    let error = error.with_hint("Register an existing installation");
    assert_eq!(error.hint(), Some("Register an existing installation"));
    assert_eq!(
        error.to_string(),
        "not_installed: No matching runtime\nHint: Register an existing installation"
    );
    let standard_error: &dyn std::error::Error = &error;
    assert!(standard_error.source().is_none());
}
