//! Explicit administrator consent for a pinned self-helper, with original-user identity lookup.
//! 对固定自身辅助进程请求显式管理员授权，并查询原用户身份。

use crate::{
    native_process::command_line,
    state::{check_type, io_error, normalize_path},
    storage::{DirectoryLease, path_key},
};
use pyrudder_core::{Error, ErrorKind, Result};
use std::{
    ffi::OsString,
    fs::OpenOptions,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, Prefix},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_CANCELLED, ERROR_INSUFFICIENT_BUFFER, LocalFree, RPC_E_CHANGED_MODE, WAIT_OBJECT_0,
    },
    Security::{
        Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        Threading::{
            GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

/// Requests Windows administrator consent for this executable and returns its actual exit code.
/// 为当前可执行文件请求 Windows 管理员授权，并返回其真实退出码。
///
/// Only the current image's absolute path is accepted; arguments retain CRT argv semantics.
/// 仅接受当前映像的绝对路径；参数保留 CRT argv 语义。
/// The shell-facing image and working-directory paths must fit the ordinary `MAX_PATH` limit.
/// 提供给 Windows shell 的映像和工作目录路径必须满足普通 `MAX_PATH` 限制。
/// The UAC prompt remains visible, but no helper console is requested. This call waits for exit.
/// UAC 授权提示保持可见，但不请求辅助控制台；本调用等待辅助进程退出。
/// The caller must explicitly supply all original-user locations and the allowed helper command.
/// 调用方必须显式提供原用户的全部位置及允许的辅助命令。
///
/// # Errors
/// Rejects unsafe or different images, invalid arguments, cancellation, and native API failures.
/// 拒绝不安全或不同的映像、无效参数、用户取消及原生 API 失败。
pub fn run_elevated(executable: &Path, arguments: &[OsString]) -> Result<u32> {
    let executable = normalize_path(executable)?;
    let current = std::env::current_exe()
        .map_err(|_| Error::new(ErrorKind::Internal, "Cannot locate the current executable"))?;
    if path_key(&executable)? != path_key(&current)? {
        return Err(Error::new(
            ErrorKind::Usage,
            "Administrator authorization is restricted to the current executable",
        ));
    }
    let parent = executable
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::Usage, "Elevated executable requires a parent"))?;
    let _directory = DirectoryLease::acquire(parent, false)?;
    let image = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&executable)
        .map_err(|error| io_error("pin elevated executable", &executable, &error))?;
    check_type(
        &image
            .metadata()
            .map_err(|error| io_error("inspect elevated executable", &executable, &error))?,
        false,
    )?;
    let encoded = command_line(&executable, arguments)?;
    let image_prefix = command_line(&executable, &[])?;
    // The prefix includes its terminating NUL; that index skips the full command's one space.
    // Reusing the existing encoder preserves quotes, backslashes, empty and non-Unicode argv.
    // 前缀长度包含终止 NUL；此索引恰好跳过完整命令中随后的一个空格。
    // 复用已有编码器，保留引号、反斜杠、空参数和非 Unicode argv。
    let parameters = if arguments.is_empty() {
        &[0_u16][..]
    } else {
        encoded
            .get(image_prefix.len()..)
            .ok_or_else(|| Error::new(ErrorKind::Internal, "Invalid elevated argument encoding"))?
    };
    let executable = shell_path(&executable)?;
    let directory = shell_path(parent)?;
    let _apartment = ComApartment::initialize()?;
    launch_and_wait(&executable, parameters, &directory)
}

fn shell_path(value: &Path) -> Result<Vec<u16>> {
    // Keep the validated paths unchanged for identity checks and file/directory anchors.
    // Only remove the recognized verbatim-drive prefix in the separate shell-facing buffer.
    // 用于身份检查和文件/目录锚定的已验证路径保持不变。
    // 仅在单独提供给 shell 的缓冲区中移除已识别的 verbatim 盘符前缀。
    let prefix_units = match value.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) => 0,
            Prefix::VerbatimDisk(_) => 4,
            _ => {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Elevation requires a drive-letter path",
                ));
            }
        },
        _ => {
            return Err(Error::new(
                ErrorKind::Usage,
                "Elevation requires a drive-letter path",
            ));
        }
    };
    let mut encoded: Vec<_> = value
        .as_os_str()
        .encode_wide()
        .skip(prefix_units)
        .take(260)
        .collect();
    if encoded.len() >= 260 || encoded.contains(&0) {
        return Err(Error::new(
            ErrorKind::Usage,
            "Administrator launch paths cannot contain NUL and must be shorter than 260 UTF-16 units",
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

#[allow(unsafe_code)]
fn launch_and_wait(executable: &[u16], parameters: &[u16], directory: &[u16]) -> Result<u32> {
    let verb = [114_u16, 117, 110, 97, 115, 0];
    let mut information = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(std::mem::size_of::<SHELLEXECUTEINFOW>()).map_err(|_| {
            Error::new(
                ErrorKind::Internal,
                "Invalid shell execution structure size",
            )
        })?,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..SHELLEXECUTEINFOW::default()
    };
    // SAFETY: all strings are live NUL-terminated buffers; the initialized structure is writable.
    // The runas verb triggers normal UAC; no shell command language or execution-policy change occurs.
    // 安全性：所有字符串均为有效的 NUL 结尾缓冲区；已初始化结构体可写。
    // runas 动词触发常规 UAC；不使用 shell 命令语言，也不改变执行策略。
    if unsafe { ShellExecuteExW(&raw mut information) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == i32::try_from(ERROR_CANCELLED).ok() {
            return Err(Error::new(
                ErrorKind::Permission,
                "Administrator authorization was cancelled; no elevated helper was started",
            )
            .with_hint("Retry when you are ready to approve the Windows administrator prompt"));
        }
        return Err(native_error(
            "start administrator-authorized helper",
            &error,
        ));
    }
    if information.hProcess.is_null() {
        return Err(Error::new(
            ErrorKind::Internal,
            "Windows did not return the elevated helper process handle",
        ));
    }
    // SAFETY: SEE_MASK_NOCLOSEPROCESS transfers one successful launch's process handle to us.
    // 安全性：SEE_MASK_NOCLOSEPROCESS 将成功启动进程的一个句柄转交本作用域拥有。
    let process = unsafe { OwnedHandle::from_raw_handle(information.hProcess) };
    // SAFETY: waits on a valid owned process handle without changing or terminating that process.
    // 安全性：等待有效且拥有的进程句柄，不修改或终止该进程。
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
        return Err(native_error(
            "wait for administrator-authorized helper",
            &io::Error::last_os_error(),
        ));
    }
    let mut exit_code = 0;
    // SAFETY: the process has exited; the live u32 output receives its exact native exit code.
    // 安全性：进程已退出；有效 u32 输出接收其准确的原生退出码。
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut exit_code) } == 0 {
        return Err(native_error(
            "read administrator-authorized helper exit code",
            &io::Error::last_os_error(),
        ));
    }
    Ok(exit_code)
}

struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    #[allow(unsafe_code)]
    fn initialize() -> Result<Self> {
        let flags = u32::try_from(COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .map_err(|_| Error::new(ErrorKind::Internal, "Invalid COM apartment flags"))?;
        // SAFETY: reserved pointer is NULL; initialization affects only this calling thread.
        // 安全性：保留指针为 NULL；初始化仅影响当前调用线程。
        let status = unsafe { CoInitializeEx(std::ptr::null(), flags) };
        if status >= 0 {
            Ok(Self { initialized: true })
        } else if status == RPC_E_CHANGED_MODE {
            // An existing caller-owned apartment remains in place and is not uninitialized here.
            // 保留调用方已有 apartment，此处不对其执行反初始化。
            Ok(Self { initialized: false })
        } else {
            Err(Error::new(
                ErrorKind::Internal,
                format!("Cannot initialize administrator launch COM context (HRESULT {status:#x})"),
            ))
        }
    }
}

impl Drop for ComApartment {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances only this guard's successful initialization, on the same thread.
            // 安全性：仅在同一线程配对本守卫成功执行的初始化。
            unsafe { CoUninitialize() };
        }
    }
}

/// Returns the process token's original account SID without requesting elevation.
/// 返回当前进程令牌所属账户的 SID，不请求权限提升。
///
/// # Errors
/// Returns token-access, bounded allocation, or SID-conversion failures.
/// 返回令牌访问、有界分配或 SID 转换失败。
#[allow(unsafe_code)]
pub fn current_user_sid() -> Result<String> {
    let mut raw_token = std::ptr::null_mut();
    // SAFETY: queries our own process token into a live handle output; no privileges are changed.
    // 安全性：将自身进程令牌查询到有效句柄输出；不改变任何特权。
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw_token) } == 0 {
        return Err(native_error(
            "open current account token",
            &io::Error::last_os_error(),
        ));
    }
    // SAFETY: successful OpenProcessToken returns a newly owned closeable token handle.
    // 安全性：成功的 OpenProcessToken 返回新拥有且可关闭的令牌句柄。
    let token = unsafe { OwnedHandle::from_raw_handle(raw_token) };
    let mut bytes = 0;
    // SAFETY: NULL/zero requests the needed size and writes only the live size output.
    // 安全性：NULL/零大小仅查询所需容量，并写入有效大小输出。
    let sized = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            std::ptr::null_mut(),
            0,
            &raw mut bytes,
        )
    };
    let size_error = io::Error::last_os_error();
    if sized != 0 || size_error.raw_os_error() != i32::try_from(ERROR_INSUFFICIENT_BUFFER).ok() {
        return Err(native_error("size current account identity", &size_error));
    }
    let capacity = usize::try_from(bytes)
        .ok()
        .filter(|&bytes| (std::mem::size_of::<TOKEN_USER>()..=65_536).contains(&bytes))
        .ok_or_else(|| Error::new(ErrorKind::Internal, "Invalid current account identity size"))?;
    let mut storage = vec![0_usize; capacity.div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: the zeroed usize allocation has TOKEN_USER alignment and at least bytes capacity.
    // 安全性：零填充 usize 分配满足 TOKEN_USER 对齐，并具有至少 bytes 容量。
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            bytes,
            &raw mut bytes,
        )
    } == 0
    {
        return Err(native_error(
            "read current account identity",
            &io::Error::last_os_error(),
        ));
    }
    if usize::try_from(bytes).map_or(true, |length| {
        length < std::mem::size_of::<TOKEN_USER>() || length > capacity
    }) {
        return Err(Error::new(
            ErrorKind::Internal,
            "Invalid returned account identity size",
        ));
    }
    // SAFETY: the successful bounded token query initialized an aligned TOKEN_USER and its SID.
    // The allocation remains live through SID conversion.
    // 安全性：成功的有界令牌查询初始化了正确对齐的 TOKEN_USER 及 SID。
    // 分配在 SID 转换完成前保持有效。
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut raw_string = std::ptr::null_mut();
    // SAFETY: the token-owned SID is valid, and the API allocates a terminated UTF-16 result.
    // 安全性：令牌返回的 SID 有效，API 分配以 NUL 结尾的 UTF-16 结果。
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &raw mut raw_string) } == 0 {
        return Err(native_error(
            "format current account identity",
            &io::Error::last_os_error(),
        ));
    }
    let sid_string = LocalSidString(raw_string);
    if sid_string.0.is_null() {
        return Err(Error::new(
            ErrorKind::Internal,
            "Windows returned an empty SID allocation",
        ));
    }
    let mut encoded = Vec::new();
    for index in 0..256 {
        // SAFETY: the API guarantees a terminated SID string; stop at NUL before its allocation ends.
        // 安全性：API 保证 SID 字符串以 NUL 结尾；在分配结束前遇到 NUL 即停止。
        let unit = unsafe { sid_string.0.add(index).read() };
        if unit == 0 {
            return String::from_utf16(&encoded).map_err(|_| {
                Error::new(
                    ErrorKind::Internal,
                    "Windows returned an invalid account SID",
                )
            });
        }
        encoded.push(unit);
    }
    Err(Error::new(
        ErrorKind::Internal,
        "Windows account SID exceeds its length bound",
    ))
}

struct LocalSidString(*mut u16);

impl Drop for LocalSidString {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: this pointer is the sole owned allocation from ConvertSidToStringSidW.
        // 安全性：此指针是 ConvertSidToStringSidW 返回分配的唯一拥有者。
        unsafe { LocalFree(self.0.cast()) };
    }
}

fn native_error(operation: &str, error: &io::Error) -> Error {
    Error::new(
        if error.kind() == io::ErrorKind::PermissionDenied {
            ErrorKind::Permission
        } else {
            ErrorKind::Internal
        },
        format!(
            "Cannot {operation} (OS error {})",
            error.raw_os_error().unwrap_or_default()
        ),
    )
}
