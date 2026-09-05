//! Native terminal identity without shell hooks or parent-environment mutation.
//! 无需 shell hook 或修改父进程环境的原生终端身份识别。

use crate::{
    state::{WindowsStateFileSystem, io_error, normalize_path},
    storage::DirectoryLease,
};
use pyrudder_core::{
    Error, ErrorKind, Result,
    state::{MAX_SELECTION_FILE_BYTES, StateFileSystem, parse_selection_file},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    io,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, FILETIME, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            QueryFullProcessImageNameW, WaitForSingleObject,
        },
    },
};

const MAX_PROCESSES: usize = 65_536;
const MAX_ANCESTORS: usize = 128;

/// Exact persisted barrier that clears a choice and stops inheritance from outer shells.
/// 清除选择并阻止从外层 shell 继承的精确持久屏障标记。
pub const CLEARED_SELECTION: &[u8] = b"!unset\n";

/// Reads an explicit inherited override, otherwise the current terminal's persisted choice.
/// 读取显式继承的环境覆盖，否则读取当前终端持久保存的选择。
///
/// Missing session directories and unknown terminal hosts produce no override or disk writes.
/// 会话目录不存在或终端宿主未知时，不产生覆盖，也不写入磁盘。
/// Nested shells and script wrappers inherit the nearest saved ancestor choice unless cleared.
/// 嵌套 shell 和脚本包装器继承最近已保存的祖先选择，除非遇到清除屏障。
///
/// # Errors
/// Rejects unsafe directories, malformed bounded selection files, or ancestry-query failures.
/// 拒绝不安全目录、格式错误的有界选择文件及祖先查询失败。
pub fn selection_value(config_dir: &Path) -> Result<Option<OsString>> {
    if let Some(value) = std::env::var_os("PYRUDDER_VERSION") {
        return Ok(Some(value));
    }
    let sessions = normalize_path(config_dir)?.join("sessions");
    if !sessions
        .try_exists()
        .map_err(|error| io_error("inspect terminal selections", &sessions, &error))?
    {
        return Ok(None);
    }
    let _anchor = DirectoryLease::acquire(&sessions, false)?;
    for path in selection_paths(config_dir, MAX_ANCESTORS)? {
        let Some(bytes) = WindowsStateFileSystem.read_file(&path, MAX_SELECTION_FILE_BYTES)? else {
            continue;
        };
        if bytes == CLEARED_SELECTION {
            return Ok(None);
        }
        // A corrupt nearer choice is an error, never permission to use an outer shell's choice.
        // 更近选择损坏时必须报错，绝不因此使用外层 shell 的选择。
        let selection = parse_selection_file(&bytes).map_err(|error| {
            error.with_hint(format!(
                "Check terminal selection file \"{}\"",
                path.to_string_lossy().escape_debug()
            ))
        })?;
        return Ok(Some(OsString::from(selection.to_string())));
    }
    Ok(None)
}

/// Returns the nearest supported shell's PID/creation-time-scoped selection path.
/// 返回最近受支持 shell 的 PID/创建时间限定的版本选择文件路径。
///
/// `PowerShell` and CMD descendants share a selection without modifying their environment.
/// `PowerShell` 和 CMD 的后代进程共享选择，无需修改其环境变量。
/// An unrecognized, inaccessible, exited, or ambiguous ancestry returns `None`.
/// 无法识别、无法访问、已退出或存在歧义的祖先链返回 `None`。
/// No directory or selection file is created by this query.
/// 本查询不会创建目录或选择文件。
///
/// # Errors
/// Rejects an invalid configuration path or a failed/bounded-out process snapshot.
/// 拒绝无效配置路径，以及失败或超出边界的进程快照。
pub fn selection_path(config_dir: &Path) -> Result<Option<PathBuf>> {
    Ok(selection_paths(config_dir, 1)?.into_iter().next())
}

fn selection_paths(config_dir: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    let config_dir = normalize_path(config_dir)?;
    let parents = process_parents()?;
    let Some(current) = ProcessIdentity::open(std::process::id()) else {
        return Ok(Vec::new());
    };
    let mut seen = BTreeSet::from([current.pid]);
    let mut ancestors = vec![current];
    let mut paths = Vec::new();
    for _ in 0..MAX_ANCESTORS {
        let Some(child) = ancestors.last() else {
            return Ok(Vec::new());
        };
        let Some(&parent_pid) = parents.get(&child.pid) else {
            return Ok(validated_paths(&ancestors, paths));
        };
        if parent_pid == 0 {
            return Ok(validated_paths(&ancestors, paths));
        }
        if !seen.insert(parent_pid) {
            // Stop before the invalid edge without discarding validated inner shells.
            // 在无效边之前停止，不丢弃已验证的内层 shell。
            return Ok(validated_paths(&ancestors, paths));
        }
        let Some(parent) = ProcessIdentity::open(parent_pid) else {
            // Already validated inner shells remain usable when a farther ancestor exited.
            // 更远祖先已退出或不可查询时，已验证的内层 shell 仍然可用。
            return Ok(validated_paths(&ancestors, paths));
        };
        // A recycled parent PID must never select an unrelated terminal's state.
        // Tied timestamps are also ambiguous: a parent must be strictly older.
        // 父 PID 被重用时绝不能选择无关终端的状态。
        // 相同时间戳也视为有歧义：父进程必须严格早于子进程创建。
        if parent.created >= child.created {
            return Ok(validated_paths(&ancestors, paths));
        }
        let supported = parent.is_supported_shell();
        let filename = format!("{}-{:016x}.version", parent.pid, parent.created);
        ancestors.push(parent);
        if supported {
            paths.push(SelectionCandidate {
                path: config_dir.join("sessions").join(filename),
                ancestor_count: ancestors.len(),
            });
            if paths.len() >= limit {
                return Ok(validated_paths(&ancestors, paths));
            }
        }
    }
    // Do not follow an unbounded tree; validated candidates never exceed the same bound.
    // 不追踪无界进程树；已验证候选数量同样不超过这一边界。
    Ok(validated_paths(&ancestors, paths))
}

struct SelectionCandidate {
    path: PathBuf,
    ancestor_count: usize,
}

fn validated_paths(ancestors: &[ProcessIdentity], paths: Vec<SelectionCandidate>) -> Vec<PathBuf> {
    // Keep each candidate only if its entire path from the current process is still alive.
    // An exited farther launcher must not invalidate a live inner shell's own choice.
    // 仅保留从当前进程起的整条所需祖先链仍存活的候选。
    // 更远启动器退出时，不能使仍存活的内层 shell 自身的选择失效。
    let live_ancestors = ancestors
        .iter()
        .take_while(|process| live_creation_time(&process.handle) == Some(process.created))
        .count();
    paths
        .into_iter()
        .take_while(|candidate| candidate.ancestor_count <= live_ancestors)
        .map(|candidate| candidate.path)
        .collect()
}

struct ProcessIdentity {
    pid: u32,
    created: u64,
    image: PathBuf,
    handle: OwnedHandle,
}

impl ProcessIdentity {
    #[allow(unsafe_code)]
    fn open(pid: u32) -> Option<Self> {
        // SAFETY: query/wait-only access, no inherited handle, and no borrowed pointers.
        // 安全性：仅查询/等待访问、不继承句柄，且无借用指针。
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if raw.is_null() {
            return None;
        }
        // SAFETY: a successful OpenProcess returns a newly owned closeable handle.
        // 安全性：成功的 OpenProcess 返回新拥有且可关闭的句柄。
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        let created = live_creation_time(&handle)?;
        let mut image = vec![0_u16; 32_768];
        let mut units = 32_768_u32;
        // SAFETY: the writable buffer holds exactly the supplied UTF-16 capacity.
        // 安全性：可写缓冲区恰好容纳所提供的 UTF-16 容量。
        if unsafe {
            QueryFullProcessImageNameW(
                handle.as_raw_handle(),
                0,
                image.as_mut_ptr(),
                &raw mut units,
            )
        } == 0
        {
            return None;
        }
        let image = image.get(..usize::try_from(units).ok()?)?;
        if image.is_empty() || image.contains(&0) {
            return None;
        }
        let image = PathBuf::from(OsString::from_wide(image));
        if !image.is_absolute() {
            return None;
        }
        Some(Self {
            pid,
            created,
            image,
            handle,
        })
    }

    fn is_supported_shell(&self) -> bool {
        self.image
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                ["pwsh.exe", "powershell.exe", "cmd.exe"]
                    .iter()
                    .any(|shell| name.eq_ignore_ascii_case(shell))
            })
    }
}

#[allow(unsafe_code)]
fn live_creation_time(handle: &OwnedHandle) -> Option<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the owned process handle and four writable FILETIME outputs remain live.
    // 安全性：拥有的进程句柄及四个可写 FILETIME 输出均保持有效。
    if unsafe {
        GetProcessTimes(
            handle.as_raw_handle(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return None;
    }
    // SAFETY: the process handle has SYNCHRONIZE access; timeout zero never blocks.
    // GetProcessTimes' exit-time field is undefined for live processes, so do not inspect it.
    // 安全性：进程句柄具有 SYNCHRONIZE 权限；零超时绝不阻塞。
    // GetProcessTimes 对存活进程返回的退出时间未定义，因此不检查该字段。
    if unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } != WAIT_TIMEOUT {
        return None;
    }
    let created = filetime(created);
    (created != 0).then_some(created)
}

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

#[allow(unsafe_code)]
fn process_parents() -> Result<BTreeMap<u32, u32>> {
    // SAFETY: takes a read-only process snapshot with no pointers or process mutation.
    // 安全性：读取进程快照，不传入指针也不修改进程。
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(snapshot_error());
    }
    // SAFETY: a successful snapshot returns a newly owned closeable handle.
    // 安全性：成功创建的快照返回新拥有且可关闭的句柄。
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>())
            .map_err(|_| Error::new(ErrorKind::Internal, "Invalid process entry size"))?,
        ..PROCESSENTRY32W::default()
    };
    let mut parents = BTreeMap::new();
    // SAFETY: the live entry has the required size and writable storage.
    // 安全性：有效条目具有所需大小和可写存储。
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) };
    while found != 0 {
        if parents.len() >= MAX_PROCESSES
            || parents
                .insert(entry.th32ProcessID, entry.th32ParentProcessID)
                .is_some()
        {
            return Err(Error::new(
                ErrorKind::Internal,
                "Process snapshot exceeds bounds or contains duplicate identities",
            ));
        }
        // SAFETY: reuses the initialized entry and the live snapshot handle.
        // 安全性：复用已初始化条目和有效快照句柄。
        found = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) };
    }
    if io::Error::last_os_error().raw_os_error()
        != Some(i32::try_from(ERROR_NO_MORE_FILES).unwrap_or_default())
    {
        return Err(snapshot_error());
    }
    Ok(parents)
}

fn snapshot_error() -> Error {
    let error = io::Error::last_os_error();
    Error::new(
        ErrorKind::Internal,
        format!(
            "Cannot inspect terminal process ancestry (OS error {})",
            error.raw_os_error().unwrap_or_default()
        ),
    )
}
