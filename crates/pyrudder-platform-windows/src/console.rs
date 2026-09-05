//! Parent-only console event handling while a native child shares the same console.
//! 原生子进程共享控制台期间，仅作用于父进程的控制台事件处理。

use pyrudder_core::{Error, ErrorKind, Result};
use std::{
    io,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GetConsoleCP, SetConsoleCtrlHandler,
};

static WAITING: AtomicBool = AtomicBool::new(false);
static LEASE: Mutex<()> = Mutex::new(());

// The callback neither allocates nor touches borrowed state, and is never inherited by children.
// 回调不分配内存、不访问借用状态，也不会被子进程继承。
#[allow(unsafe_code)]
unsafe extern "system" fn parent_handler(event: u32) -> i32 {
    i32::from(matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) && WAITING.load(Ordering::Acquire))
}

pub(crate) struct ConsoleWaitGuard {
    _lease: MutexGuard<'static, ()>,
}

impl ConsoleWaitGuard {
    #[allow(unsafe_code)]
    pub(crate) fn install() -> Result<Option<Self>> {
        // SAFETY: GetConsoleCP takes no pointers and only queries the current console.
        // 安全性：GetConsoleCP 不接受指针，只查询当前控制台。
        if unsafe { GetConsoleCP() } == 0 {
            return Ok(None);
        }
        let lease = LEASE.try_lock().map_err(|_| {
            Error::new(ErrorKind::Busy, "Another console forwarding wait is active")
        })?;
        // SAFETY: the static callback has the required system ABI and no captured references.
        // Using a callback, rather than NULL/TRUE, avoids disabling Ctrl+C in descendants.
        // 安全性：静态回调满足系统 ABI，且不捕获引用。
        // 使用回调而非 NULL/TRUE，避免禁用后代进程中的 Ctrl+C。
        if unsafe { SetConsoleCtrlHandler(Some(parent_handler), 1) } == 0 {
            let error = io::Error::last_os_error();
            return Err(Error::new(
                ErrorKind::Internal,
                format!(
                    "Cannot install console handler (OS error {})",
                    error.raw_os_error().unwrap_or_default()
                ),
            ));
        }
        WAITING.store(true, Ordering::Release);
        Ok(Some(Self { _lease: lease }))
    }
}

impl Drop for ConsoleWaitGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        WAITING.store(false, Ordering::Release);
        // SAFETY: removes the same static callback while the exclusive lease is still held.
        // If removal fails, the inactive callback returns FALSE and no longer consumes events.
        // 安全性：仍持有独占租约时移除同一个静态回调。
        // 即使移除失败，停用的回调也返回 FALSE，不再吞掉事件。
        let _removed = unsafe { SetConsoleCtrlHandler(Some(parent_handler), 0) };
    }
}
