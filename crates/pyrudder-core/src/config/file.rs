//! Strict, versioned TOML loading, excluded from minimal shim builds.
//! 严格的带版本 TOML 加载，不进入最小 shim 构建。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{ConfigEnvironment, ConfigOverrides, Configuration, PathConfig, PathOverrides};
use crate::{Error, ErrorKind, Result, state::StateFileSystem};

/// Maximum user configuration size in bytes. / 用户配置的最大字节数。
pub const MAX_CONFIG_BYTES: usize = 65_536;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    #[serde(default)]
    paths: PathOverrides,
    #[serde(default)]
    commands: CommandPolicy,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: 1,
            paths: PathOverrides::default(),
            commands: CommandPolicy::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CommandPolicy {
    system_fallback: bool,
}

fn parse(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(Error::new(
            ErrorKind::Usage,
            "Configuration exceeds 65536 bytes",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::new(ErrorKind::Usage, "Configuration must be UTF-8"))?;
    let document: Document = toml::from_str(text.strip_prefix('\u{feff}').unwrap_or(text))
        .map_err(|_| {
            Error::new(
                ErrorKind::Usage,
                "Invalid configuration TOML, unknown field, or missing schema_version",
            )
        })?;
    if document.schema_version != 1 {
        return Err(Error::new(
            ErrorKind::Usage,
            "Unsupported configuration schema_version; expected 1",
        ));
    }
    if document.paths.config_dir.is_some() {
        return Err(Error::new(
            ErrorKind::Usage,
            "config_dir cannot redirect its own configuration file",
        )
        .with_hint("Set PYRUDDER_CONFIG_DIR or a command-line config directory instead"));
    }
    Ok(document)
}

/// Loads the selected user config and merges CLI > environment > TOML > defaults.
/// 加载选定的用户配置，合并 CLI > 环境变量 > TOML > 默认值。
///
/// Config-file location comes only from CLI, environment, or defaults; no recursive redirects.
/// 配置文件位置仅来自 CLI、环境变量或默认值；不支持递归重定向。
/// `PYRUDDER_HOME` changes the fallback base, not individually configured paths.
/// `PYRUDDER_HOME` 修改兜底根目录，不覆盖单独配置的路径。
///
/// # Errors
/// Rejects malformed files, unknown fields, unsupported schema, and invalid effective paths.
/// 拒绝损坏文件、未知字段、不支持的 schema 与无效生效路径。
pub fn load_configuration(
    fs: &impl StateFileSystem,
    environment: &ConfigEnvironment,
    cli: &ConfigOverrides,
) -> Result<Configuration> {
    let config_default = environment
        .home
        .as_ref()
        .map(|root| root.join("config"))
        .or_else(|| {
            environment
                .roaming_app_data
                .as_ref()
                .map(|root| root.join("PyRudder"))
        });
    let config_dir = select_path(
        fs,
        cli.paths.config_dir.as_deref(),
        environment.paths.config_dir.as_deref(),
        None,
        config_default.as_deref(),
    )?;
    let config_file = config_dir.join("config.toml");
    let bytes = fs.read_file(&config_file, MAX_CONFIG_BYTES)?;
    let document = bytes
        .as_ref()
        .map(|bytes| parse(bytes))
        .transpose()
        .map_err(|error| {
            error.with_hint(format!(
                "Check user configuration \"{}\"; config_dir must be set outside this file",
                config_file.to_string_lossy().escape_debug()
            ))
        })?
        .unwrap_or_default();
    let base = environment.home.clone().or_else(|| {
        environment
            .local_app_data
            .as_ref()
            .map(|root| root.join("PyRudder"))
    });
    let directory =
        |cli: &Option<PathBuf>, env: &Option<PathBuf>, file: &Option<PathBuf>, suffix| {
            let default = base.as_ref().map(|root| root.join(suffix));
            select_path(
                fs,
                cli.as_deref(),
                env.as_deref(),
                file.as_deref(),
                default.as_deref(),
            )
        };
    Ok(Configuration {
        paths: PathConfig {
            install_dir: directory(
                &cli.paths.install_dir,
                &environment.paths.install_dir,
                &document.paths.install_dir,
                "bin",
            )?,
            runtimes_dir: directory(
                &cli.paths.runtimes_dir,
                &environment.paths.runtimes_dir,
                &document.paths.runtimes_dir,
                "runtimes",
            )?,
            downloads_dir: directory(
                &cli.paths.downloads_dir,
                &environment.paths.downloads_dir,
                &document.paths.downloads_dir,
                "downloads",
            )?,
            cache_dir: directory(
                &cli.paths.cache_dir,
                &environment.paths.cache_dir,
                &document.paths.cache_dir,
                "cache",
            )?,
            temp_dir: directory(
                &cli.paths.temp_dir,
                &environment.paths.temp_dir,
                &document.paths.temp_dir,
                "temp",
            )?,
            shims_dir: directory(
                &cli.paths.shims_dir,
                &environment.paths.shims_dir,
                &document.paths.shims_dir,
                "shims",
            )?,
            config_dir,
        },
        system_fallback: cli
            .system_fallback
            .unwrap_or(document.commands.system_fallback),
        loaded_file: bytes.map(|_| config_file),
    })
}

fn select_path(
    fs: &impl StateFileSystem,
    cli: Option<&Path>,
    env: Option<&Path>,
    file: Option<&Path>,
    default: Option<&Path>,
) -> Result<PathBuf> {
    let path = cli.or(env).or(file).or(default).ok_or_else(|| {
        Error::new(
            ErrorKind::Usage,
            "Missing application-data directory and explicit path override",
        )
        .with_hint("Set PYRUDDER_HOME or explicit absolute directory overrides")
    })?;
    fs.normalize_directory(path)
}
