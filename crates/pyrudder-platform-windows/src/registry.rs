//! One atomic registry/manifest/index snapshot, serialized under a process-wide file lease.
//! 在进程间文件租约下序列化的单一原子登记/清单/索引快照。

mod wire;

use crate::{
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, invalid, path_key},
};
use pyrudder_core::{
    Result,
    commands::{CommandKey, CommandRequest, RuntimeCommandManifest},
    runtime::RuntimeRecord,
    state::StateFileSystem,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Maximum combined snapshot bytes. / 组合快照的最大字节数。
pub const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

/// A published filename with explicit request identity and byte-level ownership evidence.
/// 已发布文件名及显式请求标识、字节级所有权证据。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedShim {
    /// Physical filename, never a path. / 物理文件名，不是路径。
    pub filename: String,
    /// Bare stem or exact filename. / 裸主名称或精确文件名。
    pub request_name: String,
    /// Whether the request is exact. / 是否为精确请求。
    pub exact: bool,
    /// GUI subsystem requirement. / GUI 子系统要求。
    pub gui: bool,
    /// Expected file bytes' SHA-256. / 预期文件字节的 SHA-256。
    pub sha256: String,
}

impl PublishedShim {
    pub(crate) fn validate(&self) -> Result<()> {
        use pyrudder_core::commands::{CommandExtension, is_reserved};
        CommandKey::new(&self.filename, &crate::commands::WindowsCommandCaseMapper)?;
        let (stem, _) = CommandExtension::split(&self.filename)
            .ok_or_else(|| invalid("Unsupported published filename"))?;
        if (is_reserved(stem) && !self.filename.eq_ignore_ascii_case("pyrudder-dispatch.exe"))
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid("Invalid shim ownership record"));
        }
        self.request()?;
        Ok(())
    }

    /// Revalidates the stored request instead of trusting serialized command keys.
    /// 重新校验存储请求，不信任序列化命令键。
    ///
    /// # Errors
    /// Rejects malformed command names. / 拒绝无效命令名。
    pub fn request(&self) -> Result<CommandRequest> {
        let key = CommandKey::new(
            &self.request_name,
            &crate::commands::WindowsCommandCaseMapper,
        )?;
        Ok(if self.exact {
            CommandRequest::Exact(key)
        } else {
            CommandRequest::Bare(key)
        })
    }
}

/// Trusted shim-location contract, stored beside installed templates and generated shims.
/// 存放在已安装模板和生成 shim 旁边的可信位置契约。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    /// Contract version. / 契约版本。
    pub schema_version: u32,
    /// Durable state directory. / 持久状态目录。
    pub config_dir: PathBuf,
    /// Stable generated-entry directory. / 稳定生成入口目录。
    pub shims_dir: PathBuf,
    /// Installed manager/template directory. / 管理程序和模板安装目录。
    pub install_dir: PathBuf,
}

impl Location {
    /// Reads and validates a bounded location file beside a program or shim.
    /// 读取并校验程序或 shim 旁的有界位置文件。
    ///
    /// # Errors
    /// Rejects missing, invalid, or oversized contracts. / 拒绝缺失、无效或过大的契约。
    pub fn read(directory: &Path) -> Result<Self> {
        let bytes = WindowsStateFileSystem
            .read_file(&directory.join("pyrudder-location.json"), 65_536)?
            .ok_or_else(|| invalid("PyRudder location is missing; run setup first"))?;
        wire::decode_location(&bytes)
    }

    /// Writes a location contract, refusing to take over a different installation.
    /// 写入位置契约，拒绝接管其他安装。
    ///
    /// # Errors
    /// Rejects an existing mismatched location or write failure.
    /// 拒绝已存在但不一致的位置或写入失败。
    pub fn publish(&self, directory: &Path) -> Result<()> {
        self.validate()?;
        let _anchor = DirectoryLease::acquire(directory, true)?;
        let path = directory.join("pyrudder-location.json");
        if let Some(bytes) = WindowsStateFileSystem.read_file(&path, 65_536)? {
            if wire::decode_location(&bytes)? != *self {
                return Err(invalid("Directory belongs to another PyRudder location"));
            }
            return Ok(());
        }
        let bytes = serde_json::to_vec(self).map_err(|_| invalid("Cannot encode shim location"))?;
        WindowsStateFileSystem.write_atomic(&path, &bytes)
    }

    /// Validates absolute, disjoint program/state/shim directories.
    /// 校验绝对且不互相包含的程序、状态和 shim 目录。
    ///
    /// # Errors
    /// Rejects unsupported schemas and overlapping or invalid paths.
    /// 拒绝不支持的版本、重叠目录和无效路径。
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            return Err(invalid("Unsupported shim location schema"));
        }
        let paths =
            [&self.config_dir, &self.shims_dir, &self.install_dir].map(|path| path_key(path));
        let paths: Vec<_> = paths.into_iter().collect::<Result<_>>()?;
        for (index, left) in paths.iter().enumerate() {
            if paths
                .iter()
                .skip(index + 1)
                .any(|right| left.starts_with(right) || right.starts_with(left))
            {
                return Err(invalid(
                    "Program, config, and shim directories must not contain one another",
                ));
            }
        }
        Ok(())
    }
}

/// A coherent in-memory version of all committed runtime and routing metadata.
/// 所有已提交运行时及路由元数据的一致内存版本。
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Monotonically increasing committed generation. / 单调递增的提交代次。
    pub generation: u64,
    /// Runtime registrations. / 运行时登记。
    pub runtimes: Vec<RuntimeRecord>,
    /// Per-runtime command inventories. / 每运行时命令清单。
    pub manifests: Vec<RuntimeCommandManifest>,
    /// Current expected physical entries. / 当前期望物理入口。
    pub shims: Vec<PublishedShim>,
    /// Stale owned files awaiting safe deletion. / 等待安全删除的陈旧拥有文件。
    pub pending_cleanup: Vec<PublishedShim>,
}

impl Snapshot {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.runtimes.len() > 256
            || self.manifests.len() > 256
            || self.shims.len() > 65_536
            || self.pending_cleanup.len() > 65_536
        {
            return Err(invalid("Registry exceeds its entry limits"));
        }
        let mut ids = BTreeSet::new();
        let mut roots = BTreeSet::new();
        let mut aliases = BTreeSet::new();
        for runtime in &self.runtimes {
            if !ids.insert(runtime.id())
                || !roots.insert(path_key(runtime.root())?)
                || runtime.aliases().iter().any(|alias| !aliases.insert(alias))
            {
                return Err(invalid("Duplicate runtime ID, root, or alias in registry"));
            }
            if runtime.command_dirs().len() > 63 {
                return Err(invalid("Too many registered script directories"));
            }
            for path in runtime.command_dirs() {
                path_key(path)?;
            }
        }
        let mut seen = BTreeSet::new();
        for manifest in &self.manifests {
            if !seen.insert(manifest.runtime_id()) || !ids.contains(manifest.runtime_id()) {
                return Err(invalid("Unbound or duplicate persisted manifest"));
            }
        }
        for collection in [&self.shims, &self.pending_cleanup] {
            let mut filenames = BTreeSet::new();
            for shim in collection {
                shim.validate()?;
                let filename =
                    CommandKey::new(&shim.filename, &crate::commands::WindowsCommandCaseMapper)?;
                if !filenames.insert(filename)
                    || shim.sha256.len() != 64
                    || !shim.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(invalid("Invalid persisted shim ownership record"));
                }
                shim.request()?;
            }
        }
        Ok(())
    }
}

/// Durable registry access; readers never create directories.
/// 持久登记访问；读取者绝不创建目录。
#[derive(Clone, Debug)]
pub struct Registry {
    directory: PathBuf,
}

impl Registry {
    /// Opens a possibly uninitialized configuration directory without writing.
    /// 不写入地打开可能尚未初始化的配置目录。
    ///
    /// # Errors
    /// Rejects unsupported paths. / 拒绝不支持的路径。
    pub fn new(directory: &Path) -> Result<Self> {
        Ok(Self {
            directory: WindowsStateFileSystem.normalize_directory(directory)?,
        })
    }

    /// State directory. / 状态目录。
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Loads one bounded snapshot; absence is an empty registry, corruption is an error.
    /// 加载一个有界快照；缺失表示空登记，损坏则报错。
    ///
    /// # Errors
    /// Reports malformed schemas, invalid records, or I/O failure.
    /// 报告无效版本、记录或 I/O 失败。
    pub fn load(&self) -> Result<Snapshot> {
        let _anchor = if self
            .directory
            .try_exists()
            .map_err(|_| invalid("Cannot inspect registry directory"))?
        {
            Some(DirectoryLease::acquire(&self.directory, false)?)
        } else {
            None
        };
        let path = self.directory.join("registry.json");
        let bytes = WindowsStateFileSystem.read_file(&path, MAX_SNAPSHOT_BYTES)?;
        match bytes {
            Some(bytes) => wire::decode(&bytes),
            None => Ok(Snapshot::default()),
        }
    }

    /// Starts an exclusive read-modify-write transaction.
    /// 开始独占的读改写事务。
    ///
    /// # Errors
    /// Returns Busy for concurrent writers and propagates snapshot errors.
    /// 并发写入时返回 Busy，并传递快照错误。
    pub fn transaction(&self) -> Result<Transaction> {
        let lock = FileLease::acquire(&self.directory.join("registry.lock"), true, true)?;
        let snapshot = self.load()?;
        Ok(Transaction {
            registry: self.clone(),
            snapshot,
            _lock: lock,
        })
    }

    /// Runtime lease path derived solely from a validated runtime ID.
    /// 仅由已校验运行时 ID 派生的租约路径。
    #[must_use]
    pub fn lease_path(&self, id: &pyrudder_core::runtime::RuntimeId) -> PathBuf {
        self.directory.join("leases").join(format!("{id}.lock"))
    }
}

/// Exclusive registry transaction; dropping it before commit preserves the old snapshot.
/// 独占登记事务；提交前丢弃会保留旧快照。
pub struct Transaction {
    registry: Registry,
    /// Editable, coherent snapshot. / 可编辑的一致快照。
    pub snapshot: Snapshot,
    _lock: FileLease,
}

impl Transaction {
    /// Atomically commits all metadata together and returns the published generation.
    /// 原子提交所有元数据并返回发布代次。
    ///
    /// # Errors
    /// Rejects invalid/oversized state or a failed atomic write.
    /// 拒绝无效/过大状态或失败的原子写入。
    pub fn commit(mut self) -> Result<u64> {
        self.snapshot.generation = self
            .snapshot
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("Registry generation overflow"))?;
        let bytes = wire::encode(&self.snapshot)?;
        let _anchor = DirectoryLease::acquire(&self.registry.directory, false)?;
        WindowsStateFileSystem
            .write_atomic(&self.registry.directory.join("registry.json"), &bytes)?;
        Ok(self.snapshot.generation)
    }
}
