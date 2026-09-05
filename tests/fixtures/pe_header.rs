//! Synthetic PE32+ header bytes, not a loadable or executable program.
//! 合成 PE32+ 文件头字节，不是可加载或可执行的程序。

pub fn pe_header(subsystem: u16) -> Vec<u8> {
    let mut bytes = vec![0_u8; 1024];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128_u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664_u16.to_le_bytes());
    bytes[134..136].copy_from_slice(&1_u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240_u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&0x22_u16.to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20b_u16.to_le_bytes());
    bytes[220..222].copy_from_slice(&subsystem.to_le_bytes());
    bytes[260..264].copy_from_slice(&16_u32.to_le_bytes());
    bytes
}
