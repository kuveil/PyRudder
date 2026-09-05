//! Anchored directories, owned locks, and bounded durable storage helpers.
//! 锚定目录、拥有的锁和有界持久存储辅助函数。

use crate::state::{WindowsStateFileSystem, check_type, io_error, normalize_path};
use pyrudder_core::{Error, ErrorKind, Result, runtime::InstallationId, state::StateFileSystem};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
};

/// Keeps every existing ancestor open without delete sharing, preventing path relocation.
/// 保持每个已有祖先目录打开且不共享删除，防止路径被移动替换。
pub struct DirectoryLease {
    handles: Vec<File>,
    path: PathBuf,
}

impl DirectoryLease {
    /// Anchors a drive-letter directory, optionally creating its missing tail.
    /// 锚定盘符目录，可选创建不存在的尾部目录。
    ///
    /// # Errors
    /// Rejects reparse points, inaccessible ancestors, and invalid paths.
    /// 拒绝重解析点、不可访问祖先和无效路径。
    pub fn acquire(path: &Path, create: bool) -> Result<Self> {
        let path = normalize_path(path)?;
        let mut ancestors: Vec<_> = path.ancestors().collect();
        ancestors.reverse();
        let mut handles = Vec::new();
        for ancestor in ancestors {
            if create
                && !ancestor.try_exists().map_err(|error| {
                    io_error("inspect directory before creation", ancestor, &error)
                })?
            {
                match fs::create_dir(ancestor) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(io_error("create state directory", ancestor, &error)),
                }
            }
            let file = OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                .open(ancestor)
                .map_err(|error| io_error("anchor directory", ancestor, &error))?;
            check_type(
                &file
                    .metadata()
                    .map_err(|error| io_error("inspect directory anchor", ancestor, &error))?,
                true,
            )?;
            handles.push(file);
        }
        let path = WindowsStateFileSystem.canonical_directory(&path)?;
        Ok(Self { handles, path })
    }

    /// Canonical held directory. / 已持有目录的规范路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for DirectoryLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DirectoryLease")
            .field("path", &self.path)
            .field("ancestors", &self.handles.len())
            .finish()
    }
}

/// A persistent lock file; closing the handle releases its sharing-mode lock.
/// 持久锁文件；关闭句柄释放共享模式锁。
pub struct FileLease {
    _file: File,
    _parent: DirectoryLease,
}

impl FileLease {
    /// Acquires a shared runtime lease or exclusive mutation lease without waiting.
    /// 非等待地获取共享运行时租约或独占修改租约。
    ///
    /// # Errors
    /// Returns Busy for contention; never removes an occupied lock file.
    /// 竞争时返回 Busy，绝不删除被占用的锁文件。
    pub fn acquire(path: &Path, exclusive: bool, create: bool) -> Result<Self> {
        let path = normalize_path(path)?;
        let parent = DirectoryLease::acquire(
            path.parent()
                .ok_or_else(|| invalid("Lock requires a parent"))?,
            create,
        )?;
        let file = OpenOptions::new()
            .read(true)
            .write(exclusive || create)
            .create(create)
            .share_mode(if exclusive { 0 } else { FILE_SHARE_READ })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|error| io_error("acquire state/runtime lease", &path, &error))?;
        check_type(
            &file
                .metadata()
                .map_err(|error| io_error("inspect lease", &path, &error))?,
            false,
        )?;
        Ok(Self {
            _file: file,
            _parent: parent,
        })
    }
}

/// Generates an OS-random nonzero installation identity.
/// 生成操作系统随机非零安装标识。
///
/// # Errors
/// Reports an unavailable system RNG. / 报告系统随机源不可用。
#[allow(unsafe_code)]
pub fn new_identity() -> Result<InstallationId> {
    let mut bytes = [0_u8; 16];
    // SAFETY: the 16-byte output is writable; a null algorithm handle selects the OS RNG.
    // 安全性：16 字节输出可写；空算法句柄选择系统随机源。
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            16,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(Error::new(
            ErrorKind::Internal,
            "System random number generator failed",
        ));
    }
    InstallationId::new(u128::from_le_bytes(bytes))
}

/// Hashes a bounded ordinary file without following reparse points.
/// 对有界普通文件计算摘要，不跟随重解析点。
///
/// # Errors
/// Reports unsafe paths, oversized files, or read failures.
/// 报告不安全路径、过大文件或读取失败。
pub fn sha256_file(path: &Path, maximum: u64) -> Result<String> {
    let path = normalize_path(path)?;
    let _parent = DirectoryLease::acquire(
        path.parent()
            .ok_or_else(|| invalid("File requires a parent"))?,
        false,
    )?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| io_error("open digest input", &path, &error))?;
    check_type(
        &file
            .metadata()
            .map_err(|error| io_error("inspect digest input", &path, &error))?,
        false,
    )?;
    let mut input = file.take(maximum.saturating_add(1));
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 65_536];
    let mut size = 0_u64;
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| io_error("hash file", &path, &error))?;
        if read == 0 {
            break;
        }
        size += read as u64;
        if size > maximum {
            return Err(invalid("File exceeds the digest size limit"));
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Windows case-insensitive normalized directory key. / Windows 大小写不敏感规范目录键。
///
/// # Errors
/// Rejects invalid or non-Unicode paths. / 拒绝无效或非 Unicode 路径。
pub fn path_key(path: &Path) -> Result<PathBuf> {
    let path = normalize_path(path)?;
    let text = path
        .to_str()
        .ok_or_else(|| invalid("Paths must be Unicode"))?;
    Ok(PathBuf::from(crate::casing::uppercase(
        text.strip_prefix(r"\\?\").unwrap_or(text),
    )?))
}

pub(crate) fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::Usage, message)
}
