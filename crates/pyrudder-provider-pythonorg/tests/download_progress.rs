//! Public progress contract checks for verified cache reuse and failures.
//! 已校验缓存复用和失败场景的公开进度契约检查。

#![cfg(windows)]

use pyrudder_provider_pythonorg::{DownloadPhase, PythonOrgProvider, Release};
use sha2::{Digest, Sha256};
use std::{error::Error, fs};

fn release_for(bytes: &[u8]) -> Release {
    Release {
        version: "3.13.12".into(),
        url: "https://www.python.org/ftp/python/3.13.12/python-3.13.12-amd64.zip".into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

#[test]
fn offline_cache_emits_verification_before_successful_cache_reuse() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let bytes = b"verified archive fixture";
    let release = release_for(bytes);
    let archive = temporary.path().join(format!("{}.zip", release.sha256));
    fs::write(&archive, bytes)?;
    let mut events = Vec::new();
    let result =
        PythonOrgProvider::download_with_progress(&release, temporary.path(), true, |event| {
            events.push(event);
        })?;
    assert_eq!(result, archive);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].phase, DownloadPhase::Verifying);
    assert_eq!(events[1].phase, DownloadPhase::Cached);
    assert!(events.iter().all(|event| {
        event.attempt == 0
            && event.downloaded == bytes.len() as u64
            && event.total == Some(bytes.len() as u64)
    }));
    assert_eq!(
        PythonOrgProvider::download(&release, temporary.path(), true)?,
        archive
    );
    Ok(())
}

#[test]
fn corrupt_cache_never_reports_success_or_removes_the_file() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let release = release_for(b"expected archive");
    let archive = temporary.path().join(format!("{}.zip", release.sha256));
    fs::write(&archive, b"corrupt archive")?;
    let mut events = Vec::new();
    let result =
        PythonOrgProvider::download_with_progress(&release, temporary.path(), true, |event| {
            events.push(event);
        });
    assert!(result.is_err());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].phase, DownloadPhase::Verifying);
    assert_eq!(fs::read(archive)?, b"corrupt archive");
    Ok(())
}

#[test]
fn offline_cache_miss_reports_no_transfer_or_completion() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let release = release_for(b"missing archive");
    let mut events = Vec::new();
    let result =
        PythonOrgProvider::download_with_progress(&release, temporary.path(), true, |event| {
            events.push(event);
        });
    assert!(result.is_err());
    assert!(events.is_empty());
    Ok(())
}

#[test]
#[ignore = "Downloads a signed official archive to verify real progress and offline reuse / 下载有签名的官方归档，验证真实进度与离线复用"]
fn live_official_archive_reports_verified_completion_and_offline_reuse()
-> Result<(), Box<dyn Error>> {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or_else(|| std::io::Error::other("Cannot locate workspace target directory"))?
        .join("target");
    let temporary = tempfile::Builder::new()
        .prefix("download-progress-live-")
        .tempdir_in(target)?;
    let cache = temporary.path().join("cache");
    let releases = PythonOrgProvider::releases(&cache, false)?;
    let release = PythonOrgProvider::select(&releases, "3.13")?;
    let mut events = Vec::new();
    let archive = PythonOrgProvider::download_with_progress(&release, &cache, false, |event| {
        events.push(event);
    })?;
    let size = fs::metadata(&archive)?.len();
    assert_eq!(
        events.first().map(|event| event.phase),
        Some(DownloadPhase::Connecting)
    );
    assert!(
        events
            .iter()
            .filter(|event| event.phase == DownloadPhase::Downloading)
            .count()
            > 2
    );
    assert_eq!(events[events.len() - 2].phase, DownloadPhase::Verifying);
    assert_eq!(
        events.last().map(|event| event.phase),
        Some(DownloadPhase::Complete)
    );
    assert_eq!(events.last().map(|event| event.downloaded), Some(size));
    assert_eq!(
        pyrudder_platform_windows::storage::sha256_file(&archive, 512 * 1024 * 1024)?,
        release.sha256
    );
    println!(
        "Official Python {}: {} bytes, {} progress events, digest verified",
        release.version,
        size,
        events.len()
    );
    events.clear();
    let reused = PythonOrgProvider::download_with_progress(&release, &cache, true, |event| {
        events.push(event);
    })?;
    assert_eq!(reused, archive);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].phase, DownloadPhase::Verifying);
    assert_eq!(events[1].phase, DownloadPhase::Cached);
    assert_eq!(events[1].downloaded, size);
    Ok(())
}
