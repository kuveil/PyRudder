//! Platform-independent domain types for `PyRudder`.
//! `PyRudder` 的平台无关领域类型。

pub mod commands;
pub mod config;
pub mod error;
pub mod resolver;
pub mod runtime;
pub mod selector;
pub mod state;
pub mod version;

pub use error::{Error, ErrorKind, Result};

/// Human-readable product name.
/// 供用户阅读的产品名称。
pub const PRODUCT_NAME: &str = "PyRudder";
