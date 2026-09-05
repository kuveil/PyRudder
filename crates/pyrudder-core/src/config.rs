//! Configuration types and source precedence.
//! 配置类型与配置来源优先级。

use std::{ffi::OsString, path::PathBuf};

#[cfg(feature = "config-toml")]
pub mod file;

/// Optional path settings; precedence is CLI, environment, file, then defaults.
/// 可选路径设置；优先级为 CLI、环境变量、文件、默认值。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "config-toml", derive(serde::Deserialize))]
#[cfg_attr(feature = "config-toml", serde(default, deny_unknown_fields))]
pub struct PathOverrides {
    /// Program directory. / 程序目录。
    pub install_dir: Option<PathBuf>,
    /// Managed runtimes directory. / 托管运行时目录。
    pub runtimes_dir: Option<PathBuf>,
    /// Downloaded artifacts directory. / 下载产物目录。
    pub downloads_dir: Option<PathBuf>,
    /// Rebuildable cache directory. / 可重建缓存目录。
    pub cache_dir: Option<PathBuf>,
    /// Temporary data directory. / 临时数据目录。
    pub temp_dir: Option<PathBuf>,
    /// Stable shim directory. / 固定 shim 目录。
    pub shims_dir: Option<PathBuf>,
    /// Configuration location, allowed only from CLI or environment to avoid recursive lookup.
    /// 配置位置，仅允许 CLI 或环境变量设置，避免递归查找。
    pub config_dir: Option<PathBuf>,
}

/// An injectable environment snapshot; no process-global mutation is needed for tests.
/// 可注入的环境快照；测试不需要修改进程全局环境。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigEnvironment {
    /// Windows local application-data directory. / Windows 本地应用数据目录。
    pub local_app_data: Option<PathBuf>,
    /// Windows roaming application-data directory. / Windows 漫游应用数据目录。
    pub roaming_app_data: Option<PathBuf>,
    /// Optional base for defaults; individually configured paths still take precedence.
    /// 可选默认根目录；单独配置的路径仍具有更高优先级。
    pub home: Option<PathBuf>,
    /// Individual path environment overrides. / 单独路径的环境变量覆盖。
    pub paths: PathOverrides,
    /// Raw current-shell selection; validate only if this source is reached.
    /// 原始当前终端选择；仅在解析到此来源时校验。
    pub version: Option<OsString>,
}

impl ConfigEnvironment {
    /// Captures only documented variables without modifying the current environment.
    /// 只捕获文档约定的变量，不修改当前环境。
    #[must_use]
    pub fn capture() -> Self {
        let path = |key| std::env::var_os(key).map(PathBuf::from);
        Self {
            local_app_data: path("LOCALAPPDATA"),
            roaming_app_data: path("APPDATA"),
            home: path("PYRUDDER_HOME"),
            paths: PathOverrides {
                install_dir: path("PYRUDDER_INSTALL_DIR"),
                runtimes_dir: path("PYRUDDER_RUNTIMES_DIR"),
                downloads_dir: path("PYRUDDER_DOWNLOADS_DIR"),
                cache_dir: path("PYRUDDER_CACHE_DIR"),
                temp_dir: path("PYRUDDER_TEMP_DIR"),
                shims_dir: path("PYRUDDER_SHIMS_DIR"),
                config_dir: path("PYRUDDER_CONFIG_DIR"),
            },
            version: std::env::var_os("PYRUDDER_VERSION"),
        }
    }
}

/// Trusted command-line overrides; project selection files cannot provide these settings.
/// 可信命令行覆盖；项目选择文件不能提供这些设置。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    /// Optional path overrides. / 可选路径覆盖。
    pub paths: PathOverrides,
    /// Optional system-fallback policy override. / 可选系统回退策略覆盖。
    pub system_fallback: Option<bool>,
}

/// Effective configuration; loading does not create directories or modify PATH.
/// 生效配置；加载过程不创建目录或修改 PATH。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Configuration {
    /// Normalized effective directories. / 规范化后的生效目录。
    pub paths: PathConfig,
    /// Whether system intent may be resolved; dispatch must still filter PATH.
    /// 是否允许解析系统请求；分派时仍须过滤 PATH。
    pub system_fallback: bool,
    /// The user configuration file actually loaded, if any. / 实际加载的可选用户配置文件。
    pub loaded_file: Option<PathBuf>,
}

/// Independently configurable filesystem locations used by `PyRudder`.
/// `PyRudder` 使用的可独立配置文件系统位置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathConfig {
    /// Directory containing the `PyRudder` program binaries.
    /// 包含 `PyRudder` 程序二进制文件的目录。
    pub install_dir: PathBuf,
    /// Default destination for managed Python runtimes.
    /// 托管 Python 运行时的默认目标目录。
    pub runtimes_dir: PathBuf,
    /// Directory containing complete downloaded artifacts.
    /// 保存完整下载产物的目录。
    pub downloads_dir: PathBuf,
    /// Directory containing rebuildable metadata and HTTP cache data.
    /// 保存可重建元数据和 HTTP 缓存数据的目录。
    pub cache_dir: PathBuf,
    /// Directory containing temporary download data.
    /// 保存临时下载数据的目录。
    pub temp_dir: PathBuf,
    /// Stable command-entry directory placed on `PATH`.
    /// 加入 `PATH` 的稳定命令入口目录。
    pub shims_dir: PathBuf,
    /// Directory containing user configuration and durable state.
    /// 保存用户配置和持久状态的目录。
    pub config_dir: PathBuf,
}
