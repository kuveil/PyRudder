//! Non-linguistic Windows casing, isolated from the platform-independent model.
//! 非语言学 Windows 大小写处理，与平台无关模型隔离。

use pyrudder_core::{Error, ErrorKind, Result};
use windows_sys::Win32::Globalization::{LCMAP_UPPERCASE, LCMapStringEx};

pub(crate) fn uppercase(name: &str) -> Result<String> {
    let input: Vec<u16> = name.encode_utf16().take(32_768).collect();
    String::from_utf16(&uppercase_wide(&input)?)
        .map_err(|_| Error::new(ErrorKind::Internal, "Windows returned invalid Unicode"))
}

// Native environment names may contain unpaired UTF-16 surrogates; preserve those code units.
// 原生环境变量名可能包含未配对的 UTF-16 代理项；原样保留这些编码单元。
// Only this production wrapper permits unsafe FFI; sizes and buffers are checked locally.
// 生产代码只有本包装函数允许不安全 FFI；大小和缓冲区均在本地检查。
#[allow(unsafe_code)]
pub(crate) fn uppercase_wide(input: &[u16]) -> Result<Vec<u16>> {
    if input.is_empty() || input.len() > 32_767 || input.contains(&0) {
        return Err(Error::new(
            ErrorKind::Usage,
            "Invalid Windows case-mapping input",
        ));
    }
    let count = i32::try_from(input.len())
        .map_err(|_| Error::new(ErrorKind::Usage, "Case-mapping input is too large"))?;
    let locale = [0_u16];
    // SAFETY: locale is terminated; input is alive for count UTF-16 units; null output
    // with zero capacity queries size. Reserved pointers and flags follow the API contract.
    // 安全性：区域名以零结尾；输入在 count 个 UTF-16 单元内有效；空输出和零容量查询大小。
    // 保留指针和标志遵守 API 契约。
    let needed = unsafe {
        LCMapStringEx(
            locale.as_ptr(),
            LCMAP_UPPERCASE,
            input.as_ptr(),
            count,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if needed != count {
        return Err(Error::new(
            ErrorKind::Internal,
            "Windows case-mapping size query failed",
        ));
    }
    let size = usize::try_from(needed)
        .map_err(|_| Error::new(ErrorKind::Internal, "Invalid Windows mapped size"))?;
    let mut output = vec![0_u16; size];
    // SAFETY: buffers are distinct, live, and sized in UTF-16 units; the destination has
    // exactly the capacity obtained above. No pointer escapes this synchronous call.
    // 安全性：缓冲区独立且有效，长度单位为 UTF-16；输出容量等于前述查询结果。
    // 指针不会逃逸出本同步调用。
    let written = unsafe {
        LCMapStringEx(
            locale.as_ptr(),
            LCMAP_UPPERCASE,
            input.as_ptr(),
            count,
            output.as_mut_ptr(),
            needed,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if written != needed {
        return Err(Error::new(
            ErrorKind::Internal,
            "Windows case mapping failed",
        ));
    }
    // Ordinal Windows comparison uses UTF-16 code units, not supplementary-letter casing.
    // Preserve surrogate units that LCMapStringEx may otherwise map as whole Unicode scalars.
    // Windows 序数比较使用 UTF-16 编码单元，而不是补充平面字母的大小写映射。
    // 保留代理编码单元，避免 LCMapStringEx 将其按完整 Unicode 标量映射。
    for (original, mapped) in input.iter().zip(&mut output) {
        if (0xd800..=0xdfff).contains(original) {
            *mapped = *original;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::uppercase;
    use pyrudder_core::{Error, ErrorKind, Result};
    use windows_sys::Win32::Globalization::CompareStringOrdinal;

    #[allow(unsafe_code)]
    fn ordinal_equal(left: &str, right: &str) -> Result<bool> {
        let left: Vec<u16> = left.encode_utf16().collect();
        let right: Vec<u16> = right.encode_utf16().collect();
        let left_len = i32::try_from(left.len())
            .map_err(|_| Error::new(ErrorKind::Internal, "Test input too large"))?;
        let right_len = i32::try_from(right.len())
            .map_err(|_| Error::new(ErrorKind::Internal, "Test input too large"))?;
        // SAFETY: both vectors live through this synchronous read-only call; lengths are exact.
        // 安全性：两个向量在同步只读调用期间有效；长度准确。
        let result =
            unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
        match result {
            1 | 3 => Ok(false),
            2 => Ok(true),
            _ => Err(Error::new(
                ErrorKind::Internal,
                "Windows ordinal comparison failed",
            )),
        }
    }

    #[test]
    fn filesystem_keys_agree_with_native_ordinal_equality_for_bmp_case_pairs() -> Result<()> {
        for value in (1..=0xffff).chain(0x10400..=0x1044f) {
            let Some(ch) = char::from_u32(value) else {
                continue;
            };
            let original = ch.to_string();
            let mapped = uppercase(&original)?;
            for variant in [
                ch.to_uppercase().collect::<String>(),
                ch.to_lowercase().collect::<String>(),
            ] {
                assert_eq!(
                    mapped == uppercase(&variant)?,
                    ordinal_equal(&original, &variant)?,
                    "U+{value:04X}"
                );
            }
            assert_eq!(uppercase(&mapped)?, mapped);
        }
        Ok(())
    }
}
