//! Python runtime identity, registration metadata, and ownership types.
//! Python 运行时标识、登记元数据与所有权类型。

use std::{
    collections::BTreeSet,
    fmt,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use crate::{Result, error::invalid_input, version::PythonVersion};

/// Maximum runtime ID length in bytes. / 运行时 ID 的最大字节数。
pub const MAX_RUNTIME_ID_LENGTH: usize = 128;

/// Python implementation supported by the MVP. / MVP 支持的 Python 实现。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Implementation {
    /// Standard `CPython`. / 标准 `CPython` 实现。
    Cpython,
}

/// Processor architecture supported by the MVP. / MVP 支持的处理器架构。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Architecture {
    /// 64-bit x86. / 64 位 x86 架构。
    X64,
}

/// Build variant, separate from the version's release phase.
/// 构建变体，与版本的发布阶段分开。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeVariant {
    /// Standard build (not debug or free-threaded). / 标准构建（非调试或自由线程构建）。
    Standard,
}

/// A nonzero 128-bit installation identifier, displayed as 32 lowercase hex digits.
/// 非零的 128 位安装标识，显示为 32 位小写十六进制数字。
///
/// The registry must generate IDs and check uniqueness; this type provides no randomness.
/// 注册表负责生成标识和校验唯一性；此类型不提供随机数生成能力。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstallationId(u128);

impl InstallationId {
    /// Constructs an identifier from a nonzero value. / 从非零值构建标识。
    ///
    /// # Errors
    /// Rejects zero, which is reserved for missing IDs. / 拒绝表示缺失标识的零值。
    pub fn new(value: u128) -> Result<Self> {
        if value == 0 {
            return Err(invalid_input("Installation ID must not be zero"));
        }
        Ok(Self(value))
    }

    /// Returns the numeric identifier. / 返回数字形式的标识。
    #[must_use]
    pub const fn value(self) -> u128 {
        self.0
    }
}

impl FromStr for InstallationId {
    type Err = crate::Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.len() != 32 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid_input(
                "Installation ID requires exactly 32 hexadecimal digits",
            ));
        }
        let value = u128::from_str_radix(input, 16)
            .map_err(|_| invalid_input("Invalid installation ID"))?;
        Self::new(value)
    }
}

impl fmt::Display for InstallationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// A validated exact runtime ID; an optional suffix distinguishes installations.
/// 合法的精确运行时 ID；可选后缀用于区分多个安装。
///
/// Canonical format: `cpython-M.m.p-x64[@installation-id]`. Unqualified IDs identify
/// the canonical managed installation; external registrations require a suffix.
/// 规范格式：`cpython-M.m.p-x64[@installation-id]`。无后缀标识规范托管安装；
/// 外部安装登记必须具有后缀。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuntimeId {
    implementation: Implementation,
    version: PythonVersion,
    architecture: Architecture,
    variant: RuntimeVariant,
    installation: Option<InstallationId>,
}

impl RuntimeId {
    /// Creates a standard `CPython` x64 runtime ID. / 创建标准 `CPython` x64 运行时 ID。
    ///
    /// # Errors
    /// The MVP rejects prerelease runtimes. / MVP 拒绝预发布运行时。
    pub fn new(version: PythonVersion) -> Result<Self> {
        if !version.is_stable() {
            return Err(invalid_input(
                "The MVP supports stable CPython x64 runtimes only",
            ));
        }
        Ok(Self {
            implementation: Implementation::Cpython,
            version,
            architecture: Architecture::X64,
            variant: RuntimeVariant::Standard,
            installation: None,
        })
    }

    /// Qualifies this ID for a specific installation. / 将 ID 限定到具体安装。
    #[must_use]
    pub fn with_installation(mut self, installation: InstallationId) -> Self {
        self.installation = Some(installation);
        self
    }

    /// Returns the Python implementation. / 返回 Python 实现。
    #[must_use]
    pub const fn implementation(&self) -> Implementation {
        self.implementation
    }

    /// Returns the exact numeric version. / 返回精确数字版本。
    #[must_use]
    pub const fn version(&self) -> PythonVersion {
        self.version
    }

    /// Returns the target architecture. / 返回目标架构。
    #[must_use]
    pub const fn architecture(&self) -> Architecture {
        self.architecture
    }

    /// Returns the build variant. / 返回构建变体。
    #[must_use]
    pub const fn variant(&self) -> RuntimeVariant {
        self.variant
    }

    /// Returns the installation qualifier, if present. / 返回可选的安装限定标识。
    #[must_use]
    pub const fn installation(&self) -> Option<InstallationId> {
        self.installation
    }
}

impl FromStr for RuntimeId {
    type Err = crate::Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.len() > MAX_RUNTIME_ID_LENGTH || !input.is_ascii() {
            return Err(invalid_input(
                "Runtime ID must be ASCII and at most 128 bytes",
            ));
        }
        let normalized = input.to_ascii_lowercase();
        let (base, installation) = match normalized.split_once('@') {
            Some((base, qualifier)) => (base, Some(qualifier.parse::<InstallationId>()?)),
            None => (normalized.as_str(), None),
        };
        let version = base
            .strip_prefix("cpython-")
            .and_then(|value| value.strip_suffix("-x64"))
            .ok_or_else(|| {
                invalid_input(
                    "Expected cpython-major.minor.patch-x64 with an optional @installation-id",
                )
            })?;
        let mut id = Self::new(version.parse()?)?;
        id.installation = installation;
        Ok(id)
    }
}

impl fmt::Display for RuntimeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "cpython-{}-x64", self.version)?;
        if let Some(installation) = self.installation {
            write!(formatter, "@{installation}")?;
        }
        Ok(())
    }
}

/// Maximum alias length in bytes. / 别名的最大字节数。
pub const MAX_ALIAS_LENGTH: usize = 64;

/// Case-insensitive ASCII alias normalized to lowercase.
/// 规范化为小写的 ASCII 大小写不敏感别名。
///
/// Grammar: `[a-z][a-z0-9_-]{0,63}`; `system`, `pyrudder`, `cpython-*` and Windows
/// device names are reserved. This is not a filesystem path or shell command.
/// 语法：`[a-z][a-z0-9_-]{0,63}`；保留 `system`、`pyrudder`、`cpython-*` 和 Windows
/// 设备名称。此类型不是文件系统路径或 shell 命令。
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuntimeAlias(String);

impl RuntimeAlias {
    /// Returns the canonical alias text. / 返回规范别名文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RuntimeAlias {
    type Err = crate::Error;

    fn from_str(input: &str) -> Result<Self> {
        if input.is_empty()
            || input.len() > MAX_ALIAS_LENGTH
            || !input.as_bytes()[0].is_ascii_alphabetic()
            || !input
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(invalid_input(
                "Alias must start with an ASCII letter and contain at most 64 letters, digits, hyphens or underscores",
            ));
        }
        let alias = input.to_ascii_lowercase();
        let numbered_device = alias
            .strip_prefix("com")
            .or_else(|| alias.strip_prefix("lpt"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
        if matches!(
            alias.as_str(),
            "system" | "pyrudder" | "con" | "prn" | "aux" | "nul"
        ) || alias.starts_with("cpython-")
            || numbered_device
        {
            return Err(invalid_input("Alias is reserved; choose another name"));
        }
        Ok(Self(alias))
    }
}

impl fmt::Display for RuntimeAlias {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Recorded provenance; metadata alone never authorizes deletion or proves trust.
/// 已记录的来源；元数据本身不构成删除授权，也不证明来源可信。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeOrigin {
    /// Files installed by `PyRudder`; uninstall also requires sentinel, boundary and lease checks.
    /// 由 `PyRudder` 安装的文件；卸载仍需校验 sentinel、目录边界及 lease。
    Managed {
        /// Provider identifier. / Provider 标识。
        provider: String,
        /// Source URL; validation belongs to the provider. / 来源 URL；由 Provider 负责校验。
        artifact_url: String,
        /// Expected SHA-256 bytes, not a verification result. / 预期 SHA-256 字节，不表示已校验。
        artifact_sha256: [u8; 32],
        /// Ownership identifier to match against the sentinel. / 用于匹配 sentinel 的所有权标识。
        ownership_id: InstallationId,
    },
    /// Files owned externally, which can only be unregistered. / 外部拥有的文件，只能注销登记。
    External {
        /// Absolute interpreter path provided during registration. / 登记时提供的绝对解释器路径。
        registered_executable: PathBuf,
    },
}

/// Last observed health; it does not replace a filesystem probe at dispatch.
/// 最近观测到的健康状态，不能替代分派时的文件系统探测。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeHealth {
    /// No health probe has completed. / 尚未完成健康探测。
    Unchecked,
    /// The last health probe passed. / 最近健康探测通过。
    Ready,
    /// Files are inconsistent or damaged. / 文件不一致或已损坏。
    Broken {
        /// Diagnostic explanation. / 诊断原因。
        reason: String,
    },
    /// The installation is temporarily inaccessible. / 安装暂时无法访问。
    Unavailable {
        /// Diagnostic explanation. / 诊断原因。
        reason: String,
    },
    /// Removal is pending; selection is disabled. / 等待删除，不可选择。
    PendingRemoval,
}

impl RuntimeHealth {
    /// Whether the last probe allows selection. / 最近探测状态是否允许选择。
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// An immutable runtime identity and paths with mutable health and alias metadata.
/// 标识与路径不可变、健康状态和别名元数据可更新的运行时记录。
///
/// Path checks here are lexical only. Canonicalization, file identity, ownership
/// verification and probing belong to registration/platform services.
/// 此处只检查路径语法。规范化、文件标识、所有权验证与探测由登记及平台服务负责。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeRecord {
    id: RuntimeId,
    root: PathBuf,
    command_dirs: Vec<PathBuf>,
    origin: RuntimeOrigin,
    health: RuntimeHealth,
    aliases: BTreeSet<RuntimeAlias>,
}

impl RuntimeRecord {
    /// Creates an unchecked record without reading or executing any files.
    /// 创建尚未探测的记录，不读取或执行任何文件。
    ///
    /// # Errors
    /// Rejects relative/parent-traversing paths, an empty script-directory list,
    /// or an external record without an installation qualifier.
    /// 拒绝相对路径、含父目录跳转的路径、空脚本目录列表和缺少安装限定标识的外部记录。
    pub fn new(
        id: RuntimeId,
        root: PathBuf,
        command_dirs: Vec<PathBuf>,
        origin: RuntimeOrigin,
    ) -> Result<Self> {
        validate_path(&root)?;
        if command_dirs.is_empty() {
            return Err(invalid_input("At least one script directory is required"));
        }
        for directory in &command_dirs {
            validate_path(directory)?;
        }
        if let RuntimeOrigin::External {
            registered_executable,
        } = &origin
        {
            validate_path(registered_executable)?;
            if id.installation().is_none() {
                return Err(invalid_input(
                    "External registrations require an @installation-id qualifier",
                ));
            }
        }
        Ok(Self {
            id,
            root,
            command_dirs,
            origin,
            health: RuntimeHealth::Unchecked,
            aliases: BTreeSet::new(),
        })
    }

    /// Returns the exact registration identity. / 返回精确登记标识。
    #[must_use]
    pub const fn id(&self) -> &RuntimeId {
        &self.id
    }

    /// Returns the recorded runtime root. / 返回登记的运行时根目录。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns script directories in discovery order. / 返回按发现顺序排列的脚本目录。
    #[must_use]
    pub fn command_dirs(&self) -> &[PathBuf] {
        &self.command_dirs
    }

    /// Returns ownership metadata, not deletion authorization. / 返回所有权元数据，不代表删除授权。
    #[must_use]
    pub const fn origin(&self) -> &RuntimeOrigin {
        &self.origin
    }

    /// Returns the last observed health. / 返回最近观测的健康状态。
    #[must_use]
    pub const fn health(&self) -> &RuntimeHealth {
        &self.health
    }

    /// Updates the health observation. / 更新健康状态记录。
    pub fn set_health(&mut self, health: RuntimeHealth) {
        self.health = health;
    }

    /// Returns the canonical aliases. / 返回规范别名集合。
    #[must_use]
    pub const fn aliases(&self) -> &BTreeSet<RuntimeAlias> {
        &self.aliases
    }

    /// Adds an alias locally; the registry must enforce uniqueness across records.
    /// 在本记录添加别名；注册表须校验跨记录唯一性。
    pub fn add_alias(&mut self, alias: RuntimeAlias) -> bool {
        self.aliases.insert(alias)
    }
}

fn validate_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(invalid_input(
            "Runtime paths must be absolute and contain no parent traversal",
        ));
    }
    Ok(())
}
