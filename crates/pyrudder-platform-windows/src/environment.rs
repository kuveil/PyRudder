//! Best-effort notification after explicit persistent environment changes.
//! 显式修改持久环境变量后进行尽力通知。

use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

/// Notifies environment consumers; existing terminal hosts still require reopening.
/// 通知环境变量使用方；现有终端宿主仍需重新打开。
pub fn notify_environment_change() {
    let environment: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the terminated UTF-16 buffer lives through the bounded synchronous call.
    // 安全性：以零结尾的 UTF-16 缓冲区在有超时的同步调用期间保持有效。
    #[allow(unsafe_code)]
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            std::ptr::null_mut(),
        );
    }
}
