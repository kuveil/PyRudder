//! Bounded PE header classification, not a loader, signature verifier, or safety check.
//! 有界 PE 文件头分类，不是加载器、签名验证器或安全检查。

use pyrudder_core::commands::{CommandKind, CommandRejection};

#[cfg(test)]
#[path = "../../../tests/fixtures/pe_header.rs"]
mod fixtures;
use std::io::{self, Read, Seek, SeekFrom};

type Classification = std::result::Result<CommandKind, CommandRejection>;

/// Reads fixed header blocks and seeks over the DOS stub; never reads the whole image.
/// 读取固定文件头块并跳过 DOS 存根；绝不读取整个映像。
pub(crate) fn classify(
    reader: &mut (impl Read + Seek),
    file_size: u64,
) -> io::Result<Classification> {
    let mut dos = [0_u8; 64];
    if !read_header(reader, &mut dos)? || &dos[..2] != b"MZ" {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    let pe_offset = u64::from(u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]));
    if pe_offset < 64 || pe_offset + 24 > file_size {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    reader.seek(SeekFrom::Start(pe_offset))?;
    let mut coff = [0_u8; 24];
    if !read_header(reader, &mut coff)? || &coff[..4] != b"PE\0\0" {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    let machine = word(&coff, 4);
    let sections = word(&coff, 6);
    let optional_size = u64::from(word(&coff, 20));
    let characteristics = word(&coff, 22);
    if !(1..=96).contains(&sections)
        || optional_size < 112
        || pe_offset + 24 + optional_size + u64::from(sections) * 40 > file_size
    {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    if characteristics & 0x0002 == 0 || characteristics & 0x2000 != 0 || machine != 0x8664 {
        return Ok(Err(CommandRejection::UnsupportedImage));
    }
    let mut optional = [0_u8; 112];
    if !read_header(reader, &mut optional)? {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    if word(&optional, 0) != 0x20b {
        return Ok(Err(CommandRejection::UnsupportedImage));
    }
    let directories = u64::from(u32::from_le_bytes([
        optional[108],
        optional[109],
        optional[110],
        optional[111],
    ]));
    if 112 + directories * 8 > optional_size {
        return Ok(Err(CommandRejection::InvalidPe));
    }
    Ok(match word(&optional, 68) {
        2 => Ok(CommandKind::PeGui),
        3 => Ok(CommandKind::PeConsole),
        _ => Err(CommandRejection::UnsupportedImage),
    })
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_header(reader: &mut impl Read, bytes: &mut [u8]) -> io::Result<bool> {
    match reader.read_exact(bytes) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, fixtures::pe_header};
    use std::io::{self, Cursor, Read, Seek, SeekFrom};

    struct CountedReader {
        data: Cursor<Vec<u8>>,
        bytes_read: usize,
        fail_after: Option<usize>,
    }

    impl Read for CountedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self
                .fail_after
                .is_some_and(|limit| self.bytes_read >= limit)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Injected header read failure",
                ));
            }
            let count = self.data.read(buffer)?;
            self.bytes_read += count;
            Ok(count)
        }
    }

    impl Seek for CountedReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.data.seek(position)
        }
    }

    #[test]
    fn all_truncated_prefixes_and_header_byte_mutations_are_bounded() -> io::Result<()> {
        let original = pe_header(3);
        for length in 0..original.len() {
            let mut reader = Cursor::new(&original[..length]);
            let _classification = classify(
                &mut reader,
                u64::try_from(length).map_err(io::Error::other)?,
            )?;
        }
        for offset in 0..432 {
            for replacement in [0, 1, 127, 255] {
                let mut bytes = original.clone();
                bytes[offset] = replacement;
                let mut reader = CountedReader {
                    data: Cursor::new(bytes),
                    bytes_read: 0,
                    fail_after: None,
                };
                let _classification = classify(&mut reader, 1024)?;
                assert!(reader.bytes_read <= 200);
            }
        }
        Ok(())
    }

    #[test]
    fn large_images_read_only_fixed_headers() -> io::Result<()> {
        let mut reader = CountedReader {
            data: Cursor::new(pe_header(3)),
            bytes_read: 0,
            fail_after: None,
        };
        assert!(classify(&mut reader, u64::MAX)?.is_ok());
        assert_eq!(reader.bytes_read, 200);
        Ok(())
    }

    #[test]
    fn io_failures_are_not_misreported_as_malformed_files() {
        let mut reader = CountedReader {
            data: Cursor::new(pe_header(3)),
            bytes_read: 0,
            fail_after: Some(64),
        };
        assert_eq!(
            classify(&mut reader, 1024).err().map(|error| error.kind()),
            Some(io::ErrorKind::PermissionDenied)
        );
    }
}
