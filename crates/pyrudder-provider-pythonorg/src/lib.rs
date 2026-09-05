//! `Python.org` runtime provider boundary.
//! `Python.org` 运行时 Provider 边界。

/// The default `Python.org` Windows runtime index.
/// 默认的 `Python.org` Windows 运行时索引。
pub const DEFAULT_INDEX_URL: &str = "https://www.python.org/ftp/python/index-windows.json";

/// Provider for trusted `Python.org` Windows runtime artifacts.
/// 可信 `Python.org` Windows 运行时产物的 Provider。
#[derive(Clone, Copy, Debug, Default)]
pub struct PythonOrgProvider;

#[cfg(windows)]
mod archive;
#[cfg(windows)]
mod feed;
#[cfg(windows)]
mod transfer;

#[cfg(windows)]
pub use archive::extract;
#[cfg(windows)]
pub use feed::Release;
#[cfg(windows)]
pub use transfer::{DownloadPhase, DownloadProgress};
