//! Handle-based file identity for immediate dispatch checks, not persisted ownership.
//! 用于即时分派检查的句柄文件身份，不是持久化所有权。

use std::{fs::File, io, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    volume: u32,
    index: u64,
}

#[allow(unsafe_code)]
pub(crate) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: File owns a live handle; the output points to properly aligned writable storage.
    // The API initializes the full structure on success; nothing reads it on failure.
    // 安全性：File 拥有有效句柄；输出指向正确对齐的可写存储。
    // API 在成功时初始化整个结构；失败时不读取结构内容。
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call above initialized this structure.
    // 安全性：上方成功的调用已初始化此结构。
    let information = unsafe { information.assume_init() };
    Ok(FileIdentity {
        volume: information.dwVolumeSerialNumber,
        index: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}
