//! Fail-closed Windows catalog verification for the official Python feed.
//! 对 Python 官方索引执行失败即拒绝的 Windows 目录签名验证。

use crate::storage::DirectoryLease;
use pyrudder_core::{Error, ErrorKind, Result};
use std::os::windows::fs::MetadataExt;
use std::{
    ffi::CStr,
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};
use windows_sys::{
    Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Security::{
            Cryptography::{
                CERT_CONTEXT, CERT_NAME_ATTR_TYPE, CTL_USAGE,
                Catalog::{
                    CRYPTCAT_OPEN_EXISTING, CryptCATAdminAcquireContext2,
                    CryptCATAdminCalcHashFromFileHandle2, CryptCATAdminReleaseContext,
                    CryptCATClose, CryptCATEnumerateMember, CryptCATOpen,
                },
                CertGetEnhancedKeyUsage, CertGetNameStringW, szOID_COMMON_NAME,
            },
            WinTrust::{
                WINTRUST_CATALOG_INFO, WINTRUST_DATA, WINTRUST_DATA_0,
                WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_CATALOG, WTD_DISABLE_MD2_MD4,
                WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, WTD_REVOKE_WHOLECHAIN,
                WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
                WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain,
                WTHelperProvDataFromStateData, WinVerifyTrust,
            },
        },
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        },
    },
    core::{GUID, w},
};

const PUBLISHER: &str = "Python Software Foundation";
const ROOT: &str = "Microsoft Identity Verification Root Certificate Authority 2020";
const PUBLISHER_EKU: &[u8] = b"1.3.6.1.4.1.311.97.608394634.79987812.305991749.578777327";
const ACTION: GUID = GUID {
    data1: 0x00aa_c56b,
    data2: 0xcd44,
    data3: 0x11d0,
    data4: [0x8c, 0xc2, 0x00, 0xc0, 0x4f, 0xc2, 0x95, 0xee],
};

/// Verify membership, OS trust, publisher, root and publisher-specific EKU.
/// 验证文件成员关系、系统信任、发布者、根证书及发布者专用 EKU。
/// Returns bytes read from the same write/delete-protected file that was verified.
/// 返回从被验证的同一个禁止写入/删除共享的文件中读取的字节。
///
/// # Errors
/// Rejects unavailable revocation data, untrusted catalogs and mismatched publishers.
/// 拒绝缺少吊销数据、不可信目录签名及发布者不匹配的情况。
#[allow(unsafe_code)]
pub fn verify_python_index(index: &Path, catalog: &Path, offline: bool) -> Result<Vec<u8>> {
    let _index_parent = DirectoryLease::acquire(index.parent().ok_or_else(failure)?, false)?;
    let _catalog_parent = DirectoryLease::acquire(catalog.parent().ok_or_else(failure)?, false)?;
    let mut file = open_input(index, 8 * 1024 * 1024)?;
    let _catalog = open_input(catalog, 4 * 1024 * 1024)?;
    let mut admin = CatalogAdmin(0);
    // SAFETY: initialized output and static algorithm; RAII releases the acquired context.
    // 安全：输出已初始化、算法为静态字符串；RAII 负责释放取得的上下文。
    if unsafe {
        CryptCATAdminAcquireContext2(&raw mut admin.0, ptr::null(), w!("SHA256"), ptr::null(), 0)
    } == 0
    {
        return Err(failure());
    }
    let mut hash = [0u8; 32];
    let mut length = 32;
    // SAFETY: live read-only file and context, exact writable hash buffer.
    // 安全：只读文件与上下文有效，摘要缓冲区可写且长度正确。
    if unsafe {
        CryptCATAdminCalcHashFromFileHandle2(
            admin.0,
            file.as_raw_handle(),
            &raw mut length,
            hash.as_mut_ptr(),
            0,
        )
    } == 0
        || length != 32
    {
        return Err(failure());
    }
    let index_name: Vec<u16> = index.as_os_str().encode_wide().chain(Some(0)).collect();
    let catalog_name: Vec<u16> = catalog.as_os_str().encode_wide().chain(Some(0)).collect();
    let tag = member_tag(&catalog_name, &hash)?;
    let mut info = WINTRUST_CATALOG_INFO {
        cbStruct: u32::try_from(size_of::<WINTRUST_CATALOG_INFO>()).map_err(|_| failure())?,
        pcwszCatalogFilePath: catalog_name.as_ptr(),
        pcwszMemberTag: tag.as_ptr(),
        pcwszMemberFilePath: index_name.as_ptr(),
        hMemberFile: file.as_raw_handle(),
        pbCalculatedFileHash: hash.as_mut_ptr(),
        cbCalculatedFileHash: length,
        hCatAdmin: admin.0,
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: u32::try_from(size_of::<WINTRUST_DATA>()).map_err(|_| failure())?,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_WHOLECHAIN,
        dwUnionChoice: WTD_CHOICE_CATALOG,
        Anonymous: WINTRUST_DATA_0 {
            pCatalog: &raw mut info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT
            | WTD_DISABLE_MD2_MD4
            | if offline {
                WTD_CACHE_ONLY_URL_RETRIEVAL
            } else {
                0
            },
        ..Default::default()
    };
    let mut action = ACTION;
    // SAFETY: all pointed-to buffers outlive verification and state closure; UI is disabled.
    // 安全：所有指针指向的缓冲区持续到验证及状态关闭完成；已禁用 UI。
    let status = unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        )
    };
    let result = if status == 0 {
        verify_signer(&data)
    } else {
        Err(Error::new(ErrorKind::Integrity, format!("Official index signature verification failed (Windows status {status:#x})"))
            .with_hint("Check Windows trust/revocation connectivity and system time. Offline mode requires previously cached certificate data; verification cannot be bypassed."))
    };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: closes only the state returned by the preceding verification call.
    // 安全：只关闭前一次验证返回的状态。
    unsafe {
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &raw mut action,
            (&raw mut data).cast(),
        );
    }
    result?;
    file.seek(SeekFrom::Start(0)).map_err(|_| failure())?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(failure());
    }
    Ok(bytes)
}

fn open_input(path: &Path, limit: u64) -> Result<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| failure())?;
    let metadata = file.metadata().map_err(|_| failure())?;
    if !metadata.is_file()
        || metadata.len() > limit
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(failure());
    }
    Ok(file)
}

#[allow(unsafe_code)]
fn member_tag(path: &[u16], hash: &[u8; 32]) -> Result<Vec<u16>> {
    // SAFETY: NUL-terminated path; catalog-owned member buffers live until closure below.
    // 安全：路径以 NUL 结尾；目录拥有的成员缓冲区持续到下方关闭目录。
    unsafe {
        let catalog = CryptCATOpen(path.as_ptr(), CRYPTCAT_OPEN_EXISTING, 0, 0, 0);
        if catalog.is_null() || catalog == INVALID_HANDLE_VALUE {
            return Err(failure());
        }
        let result = (|| {
            let mut previous = ptr::null_mut();
            for _ in 0..4096 {
                let member = CryptCATEnumerateMember(catalog, previous);
                if member.is_null() {
                    return Err(failure());
                }
                previous = member;
                let indirect = (*member).pIndirectData;
                if indirect.is_null() {
                    continue;
                }
                let digest = &(*indirect).Digest;
                let oid = (*indirect).DigestAlgorithm.pszObjId;
                if digest.cbData != 32
                    || digest.pbData.is_null()
                    || oid.is_null()
                    || CStr::from_ptr(oid.cast()).to_bytes() != b"2.16.840.1.101.3.4.2.1"
                    || std::slice::from_raw_parts(digest.pbData, 32) != hash
                {
                    continue;
                }
                let tag = (*member).pwszReferenceTag;
                if tag.is_null() {
                    return Err(failure());
                }
                let mut text = Vec::new();
                for offset in 0..4096 {
                    let unit = *tag.add(offset);
                    text.push(unit);
                    if unit == 0 {
                        return Ok(text);
                    }
                }
                return Err(failure());
            }
            Err(failure())
        })();
        CryptCATClose(catalog);
        result
    }
}

#[allow(unsafe_code)]
fn verify_signer(data: &WINTRUST_DATA) -> Result<()> {
    // SAFETY: provider-owned pointers remain valid until the caller closes WinTrust state.
    // 安全：调用方关闭 WinTrust 状态之前，提供程序持有的指针始终有效。
    unsafe {
        let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
        if provider.is_null() {
            return Err(failure());
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, 0, 0);
        if signer.is_null() || (*signer).csCertChain < 2 {
            return Err(failure());
        }
        let leaf = WTHelperGetProvCertFromChain(signer, 0);
        let root = WTHelperGetProvCertFromChain(signer, (*signer).csCertChain - 1);
        if leaf.is_null()
            || root.is_null()
            || common_name((*leaf).pCert)? != PUBLISHER
            || common_name((*root).pCert)? != ROOT
        {
            return Err(failure());
        }
        verify_eku((*leaf).pCert)
    }
}

#[allow(unsafe_code)]
fn common_name(cert: *const CERT_CONTEXT) -> Result<String> {
    if cert.is_null() {
        return Err(failure());
    }
    let mut buffer = [0u16; 512];
    // SAFETY: trusted live certificate context and bounded writable UTF-16 buffer.
    // 安全：可信且有效的证书上下文，以及有界可写 UTF-16 缓冲区。
    let size = unsafe {
        CertGetNameStringW(
            cert,
            CERT_NAME_ATTR_TYPE,
            0,
            szOID_COMMON_NAME.cast(),
            buffer.as_mut_ptr(),
            512,
        )
    };
    if !(2..512).contains(&size) {
        return Err(failure());
    }
    String::from_utf16(&buffer[..size as usize - 1]).map_err(|_| failure())
}

#[allow(unsafe_code)]
fn verify_eku(cert: *const CERT_CONTEXT) -> Result<()> {
    let mut length = 0;
    // SAFETY: live certificate, then aligned and bounded provider output storage.
    // 安全：证书有效，随后使用对齐且有界的提供程序输出存储。
    unsafe {
        if CertGetEnhancedKeyUsage(cert, 0, ptr::null_mut(), &raw mut length) == 0
            || length == 0
            || length > 65_536
        {
            return Err(failure());
        }
        let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        let usage = buffer.as_mut_ptr().cast::<CTL_USAGE>();
        if CertGetEnhancedKeyUsage(cert, 0, usage, &raw mut length) == 0
            || (*usage).cUsageIdentifier > 4096
        {
            return Err(failure());
        }
        for index in 0..(*usage).cUsageIdentifier as usize {
            let oid = *(*usage).rgpszUsageIdentifier.add(index);
            if !oid.is_null() && CStr::from_ptr(oid.cast()).to_bytes() == PUBLISHER_EKU {
                return Ok(());
            }
        }
    }
    Err(failure())
}

struct CatalogAdmin(isize);
impl Drop for CatalogAdmin {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: this RAII object uniquely owns the acquired catalog context.
            // 安全：此 RAII 对象唯一持有已取得的目录上下文。
            unsafe {
                CryptCATAdminReleaseContext(self.0, 0);
            }
        }
    }
}

fn failure() -> Error {
    Error::new(
        ErrorKind::Integrity,
        "Official index catalog, publisher, root or EKU verification failed",
    )
}
