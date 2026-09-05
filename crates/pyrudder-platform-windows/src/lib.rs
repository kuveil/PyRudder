//! Windows platform boundary for process, path, locking, and atomic-file APIs.
//! 进程、路径、锁和原子文件 API 的 Windows 平台边界。
//!
//! Unsafe `Win32` calls are isolated in small, audited platform wrappers.
//! 不安全的 `Win32` 调用隔离在经过审查的小型平台包装函数中。

#[cfg(windows)]
pub mod commands;

#[cfg(windows)]
pub mod environment;

#[cfg(windows)]
pub mod elevation;

#[cfg(windows)]
pub mod process;

#[cfg(windows)]
mod console;

#[cfg(windows)]
mod file_identity;

#[cfg(windows)]
mod native_process;

#[cfg(windows)]
mod casing;

#[cfg(windows)]
mod pe;

#[cfg(windows)]
pub mod state;

#[cfg(windows)]
pub mod storage;

#[cfg(windows)]
pub mod installation;

#[cfg(windows)]
pub mod registry;

#[cfg(windows)]
pub mod publication;

#[cfg(windows)]
pub mod probe;

#[cfg(windows)]
pub mod router;

#[cfg(windows)]
pub mod session;

#[cfg(windows)]
pub mod trust;

/// Whether the current build host is the initial supported release target.
/// 当前构建主机是否为首个受支持的发布目标。
pub const IS_SUPPORTED_HOST: bool = cfg!(all(windows, target_arch = "x86_64"));
