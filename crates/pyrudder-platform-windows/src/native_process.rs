//! Exact-image `CreateProcessW` launch with a restricted standard-handle inheritance list.
//! 使用精确映像路径和受限标准句柄继承列表的 `CreateProcessW` 启动。

use pyrudder_core::{Error, ErrorKind, Result};
use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    },
    System::{
        Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, STARTUPINFOW, TerminateProcess, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
};

const MAX_COMMAND_UNITS: usize = 32_766;

/// Builds CRT-style argv encoding without a shell, including empty and non-Unicode arguments.
/// 不经过 shell 构建 CRT 风格参数编码，包含空参数和非 Unicode 参数。
pub(crate) fn command_line(image: &Path, arguments: &[OsString]) -> Result<Vec<u16>> {
    command_line_iter(image, arguments.iter().map(OsString::as_os_str))
}

/// Encodes borrowed arguments without copying an unbounded argument array.
/// 编码借用的参数，不复制无界参数数组。
pub(crate) fn command_line_iter<'a>(
    image: &'a Path,
    arguments: impl IntoIterator<Item = &'a OsStr>,
) -> Result<Vec<u16>> {
    let mut encoded = Vec::new();
    for argument in std::iter::once(image.as_os_str()).chain(arguments) {
        if !encoded.is_empty() {
            append(&mut encoded, u16::from(b' '), 1)?;
        }
        append(&mut encoded, u16::from(b'"'), 1)?;
        let mut slashes = 0;
        for unit in argument.encode_wide() {
            if unit == 0 {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Process arguments cannot contain NUL",
                ));
            }
            if unit == u16::from(b'\\') {
                slashes += 1;
                if slashes > MAX_COMMAND_UNITS {
                    return Err(oversized());
                }
                continue;
            }
            let escaped_slashes = if unit == u16::from(b'"') {
                slashes * 2 + 1
            } else {
                slashes
            };
            append(&mut encoded, u16::from(b'\\'), escaped_slashes)?;
            append(&mut encoded, unit, 1)?;
            slashes = 0;
        }
        append(&mut encoded, u16::from(b'\\'), slashes * 2)?;
        append(&mut encoded, u16::from(b'"'), 1)?;
    }
    encoded.push(0);
    Ok(encoded)
}

fn append(output: &mut Vec<u16>, unit: u16, count: usize) -> Result<()> {
    let size = output
        .len()
        .checked_add(count)
        .filter(|&size| size <= MAX_COMMAND_UNITS)
        .ok_or_else(oversized)?;
    output.resize(size, unit);
    Ok(())
}

fn oversized() -> Error {
    Error::new(
        ErrorKind::Usage,
        "Encoded process command line exceeds the Windows limit",
    )
}

fn terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<u16> = value.encode_wide().take(32_767).collect();
    if value.contains(&0) || value.len() >= 32_767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid native process path",
        ));
    }
    value.push(0);
    Ok(value)
}

struct AttributeList<'a> {
    storage: Vec<u128>,
    _handles: std::marker::PhantomData<&'a [HANDLE]>,
}
impl<'a> AttributeList<'a> {
    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    #[allow(unsafe_code)]
    fn new(handles: &'a mut [HANDLE]) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: null with zero-sized output requests the required allocation size.
        // 安全性：空指针及零大小输出用于查询所需分配大小。
        let _sized = unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &raw mut bytes)
        };
        if bytes == 0 || bytes > 65_536 {
            return Err(io::Error::other(
                "Invalid process attribute allocation size",
            ));
        }
        let mut storage = vec![0_u128; bytes.div_ceil(std::mem::size_of::<u128>())];
        // SAFETY: the live zeroed allocation has sufficient size and 16-byte alignment.
        // 安全性：有效的零填充分配具有足够大小和 16 字节对齐。
        if unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &raw mut bytes)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut list = Self {
            storage,
            _handles: std::marker::PhantomData,
        };
        // SAFETY: the initialized list and live handle slice remain valid through CreateProcessW.
        // All handles are inheritable duplicates owned by the launch scope.
        // 安全性：已初始化列表及有效句柄切片在 CreateProcessW 期间保持有效。
        // 所有句柄均为启动作用域拥有的可继承副本。
        if unsafe {
            UpdateProcThreadAttribute(
                list.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_mut_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }
}

impl Drop for AttributeList<'_> {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the list was initialized once and is deleted before its backing allocation.
        // 安全性：列表已初始化一次，在释放其存储分配之前销毁。
        unsafe {
            DeleteProcThreadAttributeList(self.pointer());
        }
    }
}

#[allow(unsafe_code)]
fn standard_handle(which: u32) -> io::Result<Option<OwnedHandle>> {
    // SAFETY: queries a predefined standard stream and the current process pseudo-handle.
    // 安全性：查询预定义标准流和当前进程伪句柄。
    let handle = unsafe { GetStdHandle(which) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    if handle.is_null() {
        return Ok(None);
    }
    // SAFETY: GetCurrentProcess takes no pointers and returns a non-owning pseudo-handle.
    // 安全性：GetCurrentProcess 不接受指针，返回非拥有的伪句柄。
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: the source belongs to this process; output is writable; the successful duplicate
    // is newly owned, inheritable, and restricted to this launch's attribute list.
    // 安全性：来源属于当前进程；输出可写；成功时副本是新拥有的可继承句柄，
    // 并受到本次启动属性列表的限制。
    if unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &raw mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle succeeded and transferred ownership of this new handle.
    // 安全性：DuplicateHandle 已成功，当前代码拥有新句柄。
    Ok(Some(unsafe { OwnedHandle::from_raw_handle(duplicate) }))
}

/// Launches exactly the supplied image; never appends .exe, searches PATH, or invokes a shell.
/// 只启动给定映像；绝不追加 .exe、搜索 PATH 或调用 shell。
#[allow(unsafe_code)]
pub(crate) fn run(
    image: &Path,
    cwd: &Path,
    command: &mut [u16],
    environment: &mut [u16],
) -> io::Result<u32> {
    if command.is_empty()
        || command.len() > 32_767
        || command.last() != Some(&0)
        || environment.len() < 2
        || !environment.ends_with(&[0, 0])
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Native process buffers are not terminated",
        ));
    }
    let image = terminated(image.as_os_str())?;
    let cwd = terminated(cwd.as_os_str())?;
    let streams = [
        standard_handle(STD_INPUT_HANDLE)?,
        standard_handle(STD_OUTPUT_HANDLE)?,
        standard_handle(STD_ERROR_HANDLE)?,
    ];
    let mut handles: Vec<HANDLE> = streams
        .iter()
        .flatten()
        .map(AsRawHandle::as_raw_handle)
        .collect();
    let inherit_handles = !handles.is_empty();
    let mut attributes = if inherit_handles {
        Some(AttributeList::new(&mut handles)?)
    } else {
        None
    };
    // SAFETY: these Win32 POD fields permit zero values; required fields are populated below.
    // 安全性：这些 Win32 纯数据字段允许零值；下方填充必需字段。
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    let mut flags = CREATE_UNICODE_ENVIRONMENT;
    let startup_size = if let Some(attributes) = &mut attributes {
        startup.lpAttributeList = attributes.pointer();
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = streams[0]
            .as_ref()
            .map_or(std::ptr::null_mut(), AsRawHandle::as_raw_handle);
        startup.StartupInfo.hStdOutput = streams[1]
            .as_ref()
            .map_or(std::ptr::null_mut(), AsRawHandle::as_raw_handle);
        startup.StartupInfo.hStdError = streams[2]
            .as_ref()
            .map_or(std::ptr::null_mut(), AsRawHandle::as_raw_handle);
        flags |= EXTENDED_STARTUPINFO_PRESENT;
        std::mem::size_of::<STARTUPINFOEXW>()
    } else {
        std::mem::size_of::<STARTUPINFOW>()
    };
    startup.StartupInfo.cb = u32::try_from(startup_size).map_err(io::Error::other)?;
    let mut information = std::mem::MaybeUninit::<PROCESS_INFORMATION>::uninit();
    // SAFETY: all strings are terminated, command/environment are writable live buffers,
    // startup has a valid size and optional initialized attribute list, and output is writable.
    // The handle array, its owning duplicates, and the attribute allocation outlive this call.
    // 安全性：字符串均已终止，命令行/环境是有效可写缓冲区，启动结构大小正确、
    // 属性列表已初始化，输出可写。句柄数组、其拥有的副本及属性存储均活过本次调用。
    if unsafe {
        CreateProcessW(
            image.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            i32::from(inherit_handles),
            flags,
            environment.as_mut_ptr().cast(),
            cwd.as_ptr(),
            &raw const startup.StartupInfo,
            information.as_mut_ptr(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateProcessW succeeded and initialized both newly owned handles.
    // 安全性：CreateProcessW 成功并初始化了两个新拥有的句柄。
    let information = unsafe { information.assume_init() };
    // SAFETY: the two successful output handles are valid and independently owned.
    // 安全性：两个成功输出的句柄有效且独立拥有。
    let process = unsafe { OwnedHandle::from_raw_handle(information.hProcess) };
    // SAFETY: the primary thread handle is separately owned and no longer needed here.
    // 安全性：主线程句柄独立拥有，此处不再需要。
    drop(unsafe { OwnedHandle::from_raw_handle(information.hThread) });
    drop(attributes);
    drop(streams);
    // SAFETY: waits on a live owned process handle without changing console/process groups.
    // 安全性：等待有效的已拥有进程句柄，不改变控制台或进程组。
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
        let error = io::Error::last_os_error();
        // SAFETY: on an exceptional wait failure, stop only this newly created child.
        // 安全性：等待异常失败时，只终止本次新创建的子进程。
        let _terminated = unsafe { TerminateProcess(process.as_raw_handle(), 70) };
        return Err(error);
    }
    let mut code = 0;
    // SAFETY: the process handle is valid and code points to writable storage after exit.
    // 安全性：进程句柄有效，退出后的 code 指向可写存储。
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(code)
}
