//! Bounded HTTPS transfers and resumable content-addressed archive cache.
//! 有界 HTTPS 传输和可续传的内容寻址归档缓存。

use super::{
    PythonOrgProvider,
    feed::{Release, integrity},
};
use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, sha256_file},
};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, Response},
    header::{CONTENT_RANGE, ETAG, IF_RANGE, RANGE},
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;

/// Current archive operation; terminal phases require a verified SHA-256 digest.
/// 当前归档操作；结束阶段必须已通过 SHA-256 摘要校验。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadPhase {
    /// Opening an HTTP request, including bounded retries. / 发起 HTTP 请求，包含有界重试。
    Connecting,
    /// Writing received bytes into the partial archive. / 将接收的字节写入部分归档。
    Downloading,
    /// Verifying the complete archive digest. / 校验完整归档的摘要。
    Verifying,
    /// A downloaded archive was verified and published. / 下载归档已校验并发布。
    Complete,
    /// An existing cache archive was verified and reused. / 已校验并复用现有缓存归档。
    Cached,
}

/// A download checkpoint suitable for a progress bar, without provider-owned UI.
/// 适合进度条使用的下载检查点，Provider 本身不输出界面。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadProgress {
    /// Current transfer or verification phase. / 当前传输或校验阶段。
    pub phase: DownloadPhase,
    /// Bytes in this archive, including a resumed prefix; may reset on restart.
    /// 当前归档字节数，包含续传前缀；重新下载时可能归零。
    pub downloaded: u64,
    /// Complete archive length when known; absent for unknown-length responses.
    /// 已知的完整归档长度；响应长度未知时为空。
    pub total: Option<u64>,
    /// Network attempt in 1..=3, or zero when validating an existing cache entry.
    /// 网络尝试次数为 1..=3，校验已有缓存时为零。
    pub attempt: u32,
}

pub(super) fn trusted_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| integrity("Invalid provider URL"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.python.org")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/ftp/python/")
    {
        return Err(integrity(
            "Provider URLs must remain on https://www.python.org/ftp/python/",
        ));
    }
    Ok(url)
}

pub(super) fn client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("PyRudder/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || trusted_url(attempt.url().as_str()).is_err() {
                attempt.error("Redirect outside official Python origin")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| network("Cannot initialize HTTPS client"))
}

pub(super) fn get_bytes(http: &Client, url: &Url, limit: u64) -> Result<Vec<u8>> {
    let mut last = network("Cannot download signed official index");
    for _ in 0..3 {
        let attempt = || -> Result<Vec<u8>> {
            let response = http
                .get(url.clone())
                .send()
                .map_err(|_| network("Official index request failed"))?;
            if response.status() != StatusCode::OK {
                return Err(network(&format!(
                    "Official index HTTP {}",
                    response.status()
                )));
            }
            let length = response.content_length();
            if length.is_some_and(|size| size > limit) {
                return Err(integrity("Index response exceeds size limit"));
            }
            let mut bytes = Vec::new();
            response
                .take(limit + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| network("Official index response interrupted"))?;
            if bytes.len() as u64 > limit || length.is_some_and(|size| size != bytes.len() as u64) {
                return Err(integrity("Index response size mismatch"));
            }
            Ok(bytes)
        };
        match attempt() {
            Ok(bytes) => return Ok(bytes),
            Err(error) => last = error,
        }
    }
    Err(last)
}

impl PythonOrgProvider {
    /// Download or reuse an archive only after checking its signed content digest.
    /// 只有校验签名索引中的内容摘要后才下载或复用归档。
    ///
    /// # Errors
    /// Reports network, offline-cache, digest, write and lock failures.
    /// 报告网络、离线缓存、摘要、写入和锁失败。
    pub fn download(release: &Release, directory: &Path, offline: bool) -> Result<PathBuf> {
        Self::download_with_progress(release, directory, offline, |_| {})
    }

    /// Download a verified archive and report byte checkpoints, retries and cache hits.
    /// 下载已校验归档，并报告字节检查点、重试和缓存命中。
    ///
    /// Completion is reported only after verification; failed operations never emit it.
    /// 仅在校验通过后报告完成；失败操作不会发出完成事件。
    ///
    /// # Errors
    /// Reports the same networking, integrity, cache and filesystem errors as `download`.
    /// 报告与 `download` 相同的网络、完整性、缓存和文件系统错误。
    pub fn download_with_progress(
        release: &Release,
        directory: &Path,
        offline: bool,
        mut progress: impl FnMut(DownloadProgress),
    ) -> Result<PathBuf> {
        let url = trusted_url(&release.url)?;
        if release.sha256.len() != 64
            || !release.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(integrity("Invalid download digest"));
        }
        let _directory = DirectoryLease::acquire(directory, true)?;
        let _lock = FileLease::acquire(&directory.join("pythonorg-download.lock"), true, true)?;
        let target = directory.join(format!("{}.zip", release.sha256));
        if target
            .try_exists()
            .map_err(|_| network("Cannot inspect download cache"))?
        {
            let size = fs::metadata(&target)
                .map_err(|_| network("Cannot inspect download cache"))?
                .len();
            let mut checkpoint = DownloadProgress {
                phase: DownloadPhase::Verifying,
                downloaded: size,
                total: Some(size),
                attempt: 0,
            };
            progress(checkpoint);
            if sha256_file(&target, MAX_ARCHIVE)? == release.sha256 {
                checkpoint.phase = DownloadPhase::Cached;
                progress(checkpoint);
                return Ok(target);
            }
            return Err(integrity(
                "Cached archive is corrupt; remove the named cache entry explicitly before retrying",
            ));
        }
        if offline {
            return Err(network("Verified archive is unavailable in offline cache"));
        }
        let part = directory.join(format!("{}.part", release.sha256));
        let etag_path = directory.join(format!("{}.etag", release.sha256));
        let http = client()?;
        let mut last = network("Download failed");
        for attempt in 1..=3 {
            match download_part(&http, &url, &part, &etag_path, attempt, &mut progress) {
                Ok(mut checkpoint) => {
                    checkpoint.phase = DownloadPhase::Verifying;
                    progress(checkpoint);
                    if sha256_file(&part, MAX_ARCHIVE)? != release.sha256 {
                        return Err(integrity(
                            "Downloaded archive SHA-256 mismatch; partial cache retained for inspection, not installed",
                        ));
                    }
                    fs::rename(&part, &target)
                        .map_err(|_| network("Cannot publish verified archive cache"))?;
                    checkpoint.phase = DownloadPhase::Complete;
                    progress(checkpoint);
                    return Ok(target);
                }
                Err(error) => last = error,
            }
        }
        Err(last.with_hint("Partial download retained. Retry the same command to resume; HTTPS_PROXY/HTTP_PROXY/NO_PROXY are honored by the HTTP client."))
    }
}

fn download_part(
    http: &Client,
    url: &Url,
    part: &Path,
    etag_path: &Path,
    attempt: u32,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<DownloadProgress> {
    // OPEN_REPARSE_POINT and metadata validation precede truncation or writes.
    // OPEN_REPARSE_POINT 和元数据验证先于截断或写入。
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .custom_flags(0x0020_0000)
        .open(part)
        .map_err(|_| network("Cannot open partial download"))?;
    let metadata = output
        .metadata()
        .map_err(|_| network("Cannot inspect partial download"))?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > MAX_ARCHIVE
    {
        return Err(integrity("Invalid partial download file"));
    }
    let offset = metadata.len();
    progress(DownloadProgress {
        phase: DownloadPhase::Connecting,
        downloaded: offset,
        total: None,
        attempt,
    });
    let mut request = http.get(url.clone());
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
        if let Some(etag) = WindowsStateFileSystem.read_file(etag_path, 4096)? {
            if let Ok(etag) = reqwest::header::HeaderValue::from_bytes(&etag) {
                request = request.header(IF_RANGE, etag);
            }
        }
    }
    let response = request
        .send()
        .map_err(|_| network("Archive request failed"))?;
    let status = response.status();
    if status == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
        // Restart an obsolete or complete partial file on the next bounded attempt.
        // 下一次有界重试重新下载过期或已完整的部分文件。
        output
            .set_len(0)
            .map_err(|_| network("Cannot reset partial download"))?;
        return Err(network("Partial range expired; restarting download"));
    }
    let (start, total) = response_range(&response, offset)?;
    if start == 0 {
        output
            .set_len(0)
            .map_err(|_| network("Cannot restart partial download"))?;
    }
    output
        .seek(SeekFrom::Start(start))
        .map_err(|_| network("Cannot seek partial download"))?;
    if let Some(etag) = response.headers().get(ETAG) {
        if etag.as_bytes().len() <= 4096 {
            WindowsStateFileSystem.write_atomic(etag_path, etag.as_bytes())?;
        }
    }
    let expected = response.content_length();
    let checkpoint = copy_archive(
        &mut response.take(MAX_ARCHIVE - start + 1),
        &mut output,
        DownloadProgress {
            phase: DownloadPhase::Downloading,
            downloaded: start,
            total,
            attempt,
        },
        progress,
    )?;
    output
        .sync_all()
        .map_err(|_| network("Cannot flush download"))?;
    let count = checkpoint.downloaded - start;
    if count + start > MAX_ARCHIVE
        || expected.is_some_and(|length| length != count)
        || total.is_some_and(|length| length != start + count)
    {
        return Err(integrity("Archive response length mismatch"));
    }
    Ok(checkpoint)
}

fn copy_archive(
    input: &mut impl Read,
    output: &mut impl Write,
    mut checkpoint: DownloadProgress,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<DownloadProgress> {
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    progress(checkpoint);
    loop {
        let count = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(network("Archive transfer interrupted")),
        };
        let downloaded = checkpoint.downloaded + count as u64;
        if downloaded > MAX_ARCHIVE {
            return Err(integrity("Archive response exceeds size limit"));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|_| network("Cannot write partial download"))?;
        checkpoint.downloaded = downloaded;
        progress(checkpoint);
    }
    Ok(checkpoint)
}

fn response_range(response: &Response, offset: u64) -> Result<(u64, Option<u64>)> {
    match response.status() {
        StatusCode::OK => {
            if response
                .content_length()
                .is_some_and(|length| length > MAX_ARCHIVE)
            {
                return Err(integrity("Archive too large"));
            }
            Ok((0, response.content_length()))
        }
        StatusCode::PARTIAL_CONTENT if offset > 0 => {
            let value = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("bytes "))
                .ok_or_else(|| integrity("Missing resume Content-Range"))?;
            let (range, total) = value
                .split_once('/')
                .ok_or_else(|| integrity("Invalid Content-Range"))?;
            let (start, end) = range
                .split_once('-')
                .ok_or_else(|| integrity("Invalid Content-Range"))?;
            let parse = |value: &str| {
                value
                    .parse::<u64>()
                    .map_err(|_| integrity("Invalid range number"))
            };
            let (start, end, total) = (parse(start)?, parse(end)?, parse(total)?);
            if start != offset
                || end < start
                || end.checked_add(1) != Some(total)
                || total > MAX_ARCHIVE
            {
                return Err(integrity("Mismatched resume range"));
            }
            Ok((start, Some(total)))
        }
        status => Err(network(&format!("Archive HTTP {status}"))),
    }
}

fn network(message: &str) -> Error {
    Error::new(ErrorKind::Network, message)
}

#[cfg(test)]
#[path = "../tests/support/transfer_progress.rs"]
mod tests;
