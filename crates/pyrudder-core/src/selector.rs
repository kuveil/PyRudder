//! Validated selectors and deterministic selection from registered metadata.
//! 合法选择器，以及从已登记元数据中进行确定性选择的逻辑。
//!
//! This module does not read environment variables or files, probe paths, or download runtimes.
//! 此模块不读取环境变量或文件、不探测路径，也不下载运行时。

use std::{fmt, str::FromStr};

use crate::{
    Error, ErrorKind, Result,
    error::invalid_input,
    runtime::{MAX_RUNTIME_ID_LENGTH, RuntimeAlias, RuntimeId, RuntimeRecord},
    version::{PythonVersion, parse_component},
};

/// A version, exact installation, alias, or explicit system request.
/// 版本、精确安装、别名或显式系统请求。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionSelector {
    /// Latest registered patch in a minor series. / 次版本系列中最新的已登记补丁版本。
    Minor {
        /// Major version number. / 主版本号。
        major: u32,
        /// Minor version number. / 次版本号。
        minor: u32,
    },
    /// Exact version, potentially present in several installations.
    /// 精确版本，可能存在于多个安装中。
    Exact(PythonVersion),
    /// Exact registered ID, including any installation qualifier.
    /// 精确登记 ID，包括可选的安装限定标识。
    Runtime(RuntimeId),
    /// A canonical user alias. / 规范化的用户别名。
    Alias(RuntimeAlias),
    /// Explicit request for system fallback, still subject to caller policy.
    /// 显式系统回退请求，仍须受调用方策略约束。
    System,
}

/// Selection result; a system request is not a resolved executable or authorization.
/// 选择结果；系统请求并非已解析的可执行目标或执行授权。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeSelection<'a> {
    /// A uniquely selected, previously healthy registration. / 唯一选中的健康登记记录。
    Registered(&'a RuntimeRecord),
    /// The resolver must check policy and filter PATH before system fallback.
    /// Resolver 必须校验策略并过滤 PATH 后才能使用系统回退。
    SystemRequested,
}

impl VersionSelector {
    /// Selects metadata without side effects. Short versions choose the highest patch
    /// before health checks; broken or ambiguous latest versions never fall back.
    /// 无副作用地选择元数据。短版本先选最高补丁再检查健康状态；最新版本
    /// 存在损坏或歧义时不会回退。
    ///
    /// # Errors
    /// Reports unsupported versions, missing registrations, ambiguity, or unhealthy state.
    /// 返回不支持的版本、登记缺失、歧义或健康状态异常错误。
    pub fn select<'a>(&self, runtimes: &'a [RuntimeRecord]) -> Result<RuntimeSelection<'a>> {
        if matches!(self, Self::System) {
            return Ok(RuntimeSelection::SystemRequested);
        }
        if matches!(self, Self::Exact(version) if !version.is_stable()) {
            return Err(invalid_input(
                "The MVP supports stable CPython x64 runtimes only",
            ));
        }

        let mut candidates: Vec<_> = runtimes
            .iter()
            .filter(|runtime| self.matches(runtime))
            .collect();
        if matches!(self, Self::Minor { .. }) {
            if let Some(latest) = candidates
                .iter()
                .map(|runtime| runtime.id().version())
                .max()
            {
                candidates.retain(|runtime| runtime.id().version() == latest);
            }
        }

        let runtime = match candidates.as_slice() {
            [] => {
                return Err(Error::new(
                    ErrorKind::NotInstalled,
                    format!("No registered runtime matches {self}"),
                )
                .with_hint(
                    "Run pyrudder list; register or explicitly install the required runtime",
                ));
            }
            [runtime] => *runtime,
            _ => {
                let mut ids: Vec<_> = candidates
                    .iter()
                    .map(|runtime| runtime.id().to_string())
                    .collect();
                ids.sort();
                return Err(Error::new(ErrorKind::Conflict, format!("Multiple registrations match {self}: {}", ids.join(", ")))
                    .with_hint("Choose an exact runtime ID or a unique alias; repair duplicate registrations if necessary"));
            }
        };
        if !runtime.health().is_ready() {
            return Err(Error::new(
                ErrorKind::BrokenRuntime,
                format!("Selected runtime {} is not ready", runtime.id()),
            )
            .with_hint("Run pyrudder doctor and repair or unregister the runtime"));
        }
        Ok(RuntimeSelection::Registered(runtime))
    }

    fn matches(&self, runtime: &RuntimeRecord) -> bool {
        let version = runtime.id().version();
        match self {
            Self::Minor { major, minor } => version.major() == *major && version.minor() == *minor,
            Self::Exact(expected) => version == *expected,
            Self::Runtime(id) => runtime.id() == id,
            Self::Alias(alias) => runtime.aliases().contains(alias),
            Self::System => false,
        }
    }
}

impl FromStr for VersionSelector {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.is_empty() || input.len() > MAX_RUNTIME_ID_LENGTH || !input.is_ascii() {
            return Err(invalid_input(
                "Selector must be nonempty ASCII and at most 128 bytes",
            ));
        }
        if input.eq_ignore_ascii_case("system") {
            return Ok(Self::System);
        }
        if input.to_ascii_lowercase().starts_with("cpython-") {
            return input.parse().map(Self::Runtime);
        }
        if input.as_bytes()[0].is_ascii_digit() {
            if let Some((major, minor)) = input.split_once('.') {
                if !minor.contains('.') {
                    return Ok(Self::Minor {
                        major: parse_component(major)?,
                        minor: parse_component(minor)?,
                    });
                }
            }
            let version: PythonVersion = input.parse()?;
            if !version.is_stable() {
                return Err(invalid_input(
                    "The MVP supports stable CPython x64 runtimes only",
                ));
            }
            return Ok(Self::Exact(version));
        }
        input.parse().map(Self::Alias)
    }
}

impl fmt::Display for VersionSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Minor { major, minor } => write!(formatter, "{major}.{minor}"),
            Self::Exact(version) => version.fmt(formatter),
            Self::Runtime(id) => id.fmt(formatter),
            Self::Alias(alias) => alias.fmt(formatter),
            Self::System => formatter.write_str("system"),
        }
    }
}
