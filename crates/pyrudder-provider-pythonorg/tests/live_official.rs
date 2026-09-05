//! Explicit live acceptance of signed official pages and complete standard runtimes.
//! 显式验收有签名的官方分页和完整标准运行时。

#![cfg(windows)]

use pyrudder_provider_pythonorg::{PythonOrgProvider, extract};
use std::{error::Error, fs, process::Command};

#[test]
#[ignore = "Downloads and runs authenticated official Python 3.13/3.14 runtimes; needs network / 下载并执行已认证的官方 Python 3.13/3.14，需要网络"]
fn signed_feed_downloads_and_runs_supported_minors() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::Builder::new()
        .prefix("pyrudder-provider-live-")
        .tempdir()?;
    let cache = temporary.path().join("cache");
    let releases = PythonOrgProvider::releases(&cache, false)?;
    assert!(!releases.is_empty());
    let offline = PythonOrgProvider::releases(&cache, true)?;
    assert_eq!(releases.len(), offline.len());
    println!(
        "Verified online and offline feed: {} releases",
        releases.len()
    );
    for minor in ["3.13", "3.14"] {
        let release = PythonOrgProvider::select(&releases, minor)?;
        let cached_release = PythonOrgProvider::select(&offline, minor)?;
        assert_eq!(release.url, cached_release.url);
        assert_eq!(release.sha256, cached_release.sha256);
        println!("Downloading {} from {}", release.version, release.url);
        let archive = PythonOrgProvider::download(&release, &cache, false)?;
        assert_eq!(
            PythonOrgProvider::download(&release, &cache, true)?,
            archive
        );
        let staging = temporary.path().join(minor);
        fs::create_dir(&staging)?;
        extract(&archive, &release.sha256, &staging)?;
        let output = Command::new(staging.join("python.exe"))
            .args([
                "-I",
                "-c",
                "import sys,ssl,sqlite3,venv,ensurepip; print(sys.version.split()[0])",
            ])
            .output()?;
        assert!(
            output.status.success(),
            "Authenticated Python {} failed: {}",
            release.version,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            release.version
        );
        println!(
            "Verified {}, signed SHA256 {}, extraction and Python/std-lib execution passed",
            release.version, release.sha256
        );
    }
    Ok(())
}
