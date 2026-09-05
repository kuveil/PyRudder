//! Resource-bounded extraction into an exclusively created staging directory.
//! 在独占创建的 staging 目录内进行资源有界解压。

use super::feed::integrity;
use pyrudder_core::Result;
use pyrudder_platform_windows::storage::{DirectoryLease, path_key};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Extract an authenticated ZIP without links, traversal, devices or duplicate paths.
/// 解压已认证 ZIP，拒绝链接、路径穿越、设备名和重复路径。
///
/// # Errors
/// Rejects nonempty staging, unsafe entries and archive/resource/write failures.
/// 拒绝非空 staging、不安全条目，以及归档、资源、写入失败。
pub fn extract(archive: &Path, expected_sha256: &str, staging: &Path) -> Result<()> {
    let _parent = DirectoryLease::acquire(
        archive
            .parent()
            .ok_or_else(|| integrity("Archive requires parent"))?,
        false,
    )?;
    let _staging = DirectoryLease::acquire(staging, false)?;
    if fs::read_dir(staging)
        .map_err(|_| integrity("Cannot inspect staging"))?
        .next()
        .is_some()
    {
        return Err(integrity("Extraction requires empty staging"));
    }
    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x0020_0000)
        .open(archive)
        .map_err(|_| integrity("Cannot open archive"))?;
    let metadata = input
        .metadata()
        .map_err(|_| integrity("Cannot inspect archive"))?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(integrity("Archive must be an ordinary file"));
    }
    if metadata.len() > super::transfer::MAX_ARCHIVE {
        return Err(integrity("Archive size limit exceeded"));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 65_536];
    loop {
        let size = input
            .read(&mut buffer)
            .map_err(|_| integrity("Cannot hash held archive"))?;
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    if format!("{:x}", hasher.finalize()) != expected_sha256 {
        return Err(integrity("Archive changed before extraction"));
    }
    input
        .seek(SeekFrom::Start(0))
        .map_err(|_| integrity("Cannot rewind archive"))?;
    let mut zip = zip::ZipArchive::new(input).map_err(|_| integrity("Invalid runtime ZIP"))?;
    if zip.len() > 50_000 {
        return Err(integrity("ZIP entry limit exceeded"));
    }
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| integrity("Unsupported or encrypted ZIP entry"))?;
        let relative = safe_name(entry.name())?;
        let destination = staging.join(relative);
        if !names.insert(path_key(&destination)?) {
            return Err(integrity("Duplicate case-insensitive ZIP path"));
        }
        let mode = entry.unix_mode().unwrap_or(0) & 0o170_000;
        if mode != 0 && mode != 0o100_000 && mode != 0o040_000 {
            return Err(integrity("ZIP links and special files are forbidden"));
        }
        if entry.is_dir() {
            DirectoryLease::acquire(&destination, true)?;
            continue;
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| integrity("ZIP size overflow"))?;
        if entry.size() > 512 * 1024 * 1024 || total > 2 * 1024 * 1024 * 1024 {
            return Err(integrity("ZIP uncompressed size limit exceeded"));
        }
        let _destination_parent = DirectoryLease::acquire(
            destination
                .parent()
                .ok_or_else(|| integrity("ZIP entry requires parent"))?,
            true,
        )?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&destination)
            .map_err(|_| integrity("Cannot create unique ZIP destination"))?;
        let expected = entry.size();
        let copied = std::io::copy(&mut (&mut entry).take(expected + 1), &mut output)
            .map_err(|_| integrity("ZIP data, CRC or write failure"))?;
        if copied != expected {
            return Err(integrity("ZIP expanded size mismatch"));
        }
        output
            .sync_all()
            .map_err(|_| integrity("Cannot flush extracted file"))?;
    }
    Ok(())
}

fn safe_name(name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    let name = normalized.strip_suffix('/').unwrap_or(&normalized);
    if name.is_empty() || name.len() > 2048 || name.split('/').count() > 64 {
        return Err(integrity("Invalid ZIP path length/depth"));
    }
    let mut relative = PathBuf::new();
    for component in name.split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.len() > 240
            || component.ends_with(['.', ' '])
            || component
                .chars()
                .any(|ch| ch.is_control() || "<>:\"|?*".contains(ch))
        {
            return Err(integrity("Unsafe Windows ZIP path"));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }) || component.eq_ignore_ascii_case("pyrudder-owner.json")
        {
            return Err(integrity("Reserved ZIP name"));
        }
        relative.push(component);
    }
    Ok(relative)
}
