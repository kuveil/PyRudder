//! Regression checks for byte progress, resumed responses and bounded failures.
//! 字节进度、续传响应和有界失败的回归检查。

use super::{DownloadPhase, DownloadProgress, MAX_ARCHIVE, copy_archive, download_part};
use reqwest::{Url, blocking::Client};
use std::{
    error::Error,
    fs,
    io::{self, Cursor, Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::Duration,
};

type TestResult = Result<(), Box<dyn Error>>;
type ServerFixture = (Url, thread::JoinHandle<io::Result<String>>);
type TransferOutcome = (
    super::Result<DownloadProgress>,
    Vec<DownloadProgress>,
    String,
);

fn initial(offset: u64, total: Option<u64>, attempt: u32) -> DownloadProgress {
    DownloadProgress {
        phase: DownloadPhase::Downloading,
        downloaded: offset,
        total,
        attempt,
    }
}

#[test]
fn byte_progress_includes_resumed_prefix_and_preserves_attempt() -> TestResult {
    let bytes = vec![42; 150_000];
    let mut output = Vec::new();
    let mut events = Vec::new();
    let final_event = copy_archive(
        &mut Cursor::new(&bytes),
        &mut output,
        initial(10_000, Some(160_000), 2),
        &mut |event| events.push(event),
    )?;
    assert_eq!(output, bytes);
    assert_eq!(final_event.downloaded, 160_000);
    assert_eq!(events.first().map(|event| event.downloaded), Some(10_000));
    assert_eq!(events.len(), 4);
    assert!(events.iter().all(|event| {
        event.phase == DownloadPhase::Downloading
            && event.attempt == 2
            && event.total == Some(160_000)
    }));
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].downloaded < pair[1].downloaded)
    );
    Ok(())
}

#[test]
fn unknown_content_length_never_invents_a_total() -> TestResult {
    let mut output = Vec::new();
    let mut events = Vec::new();
    let final_event = copy_archive(
        &mut Cursor::new(b"unknown length body"),
        &mut output,
        initial(0, None, 1),
        &mut |event| events.push(event),
    )?;
    assert_eq!(final_event.downloaded, 19);
    assert!(events.iter().all(|event| event.total.is_none()));
    Ok(())
}

struct FailedRead;

impl Read for FailedRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "interrupted",
        ))
    }
}

#[test]
fn interrupted_transfer_retains_real_written_progress_without_completion() {
    let mut input = Cursor::new(b"received").chain(FailedRead);
    let mut output = Vec::new();
    let mut events = Vec::new();
    let result = copy_archive(&mut input, &mut output, initial(5, None, 3), &mut |event| {
        events.push(event);
    });
    assert!(result.is_err());
    assert_eq!(output, b"received");
    assert_eq!(events.last().map(|event| event.downloaded), Some(13));
    assert!(
        events
            .iter()
            .all(|event| event.phase == DownloadPhase::Downloading)
    );
}

#[test]
fn over_limit_bytes_are_not_written_or_reported_as_completed() {
    let mut output = Vec::new();
    let mut events = Vec::new();
    let result = copy_archive(
        &mut Cursor::new(b"too much"),
        &mut output,
        initial(MAX_ARCHIVE - 1, None, 1),
        &mut |event| events.push(event),
    );
    assert!(result.is_err());
    assert!(output.is_empty());
    assert_eq!(events, vec![initial(MAX_ARCHIVE - 1, None, 1)]);
}

fn serve_once(response: &'static str) -> Result<ServerFixture, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = Url::parse(&format!("http://{}/archive.zip", listener.local_addr()?))?;
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
            if stream.read(&mut byte)? == 0 {
                break;
            }
            request.push(byte[0]);
        }
        stream.write_all(response.as_bytes())?;
        Ok(String::from_utf8_lossy(&request).into_owned())
    });
    Ok((url, handle))
}

fn transfer_response(
    response: &'static str,
    part: &Path,
    attempt: u32,
) -> Result<TransferOutcome, Box<dyn Error>> {
    let (url, handle) = serve_once(response)?;
    let http = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut events = Vec::new();
    let result = download_part(
        &http,
        &url,
        part,
        &part.with_extension("etag"),
        attempt,
        &mut |event| events.push(event),
    );
    let request = handle
        .join()
        .map_err(|_| io::Error::other("HTTP fixture panicked"))??;
    Ok((result, events, request))
}

#[test]
fn resumed_response_reports_complete_archive_size_and_actual_range() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    fs::write(&part, b"abc")?;
    let (result, events, request) = transfer_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef",
        &part,
        2,
    )?;
    assert_eq!(result?, initial(6, Some(6), 2));
    assert_eq!(fs::read(part)?, b"abcdef");
    assert!(request.to_ascii_lowercase().contains("range: bytes=3-"));
    assert_eq!(events.first().map(|event| event.downloaded), Some(3));
    assert_eq!(events.last().map(|event| event.total), Some(Some(6)));
    Ok(())
}

#[test]
fn full_response_resets_a_stale_partial_progress_to_zero() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    fs::write(&part, b"old partial")?;
    let (result, events, _) = transfer_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew",
        &part,
        2,
    )?;
    assert_eq!(result?, initial(3, Some(3), 2));
    assert_eq!(fs::read(part)?, b"new");
    assert_eq!(events[0].downloaded, 11);
    assert_eq!(events[1], initial(0, Some(3), 2));
    Ok(())
}

#[test]
fn invalid_resume_range_keeps_the_partial_and_reports_no_download() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    fs::write(&part, b"abc")?;
    let (result, events, _) = transfer_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 2-4/5\r\nConnection: close\r\n\r\ndef",
        &part,
        1,
    )?;
    assert!(result.is_err());
    assert_eq!(fs::read(part)?, b"abc");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].phase, DownloadPhase::Connecting);
    Ok(())
}

#[test]
fn unknown_length_http_response_has_byte_progress_without_total() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    let (result, events, _) = transfer_response(
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcdef",
        &part,
        1,
    )?;
    assert_eq!(result?, initial(6, None, 1));
    assert!(events.iter().all(|event| event.total.is_none()));
    assert_eq!(fs::read(part)?, b"abcdef");
    Ok(())
}

#[test]
fn expired_range_restarts_next_attempt_from_zero() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    fs::write(&part, b"abc")?;
    let (failed, first_events, _) = transfer_response(
        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        &part,
        1,
    )?;
    assert!(failed.is_err());
    assert_eq!(fs::metadata(&part)?.len(), 0);
    assert_eq!(first_events[0].downloaded, 3);
    let (result, events, request) = transfer_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew",
        &part,
        2,
    )?;
    assert_eq!(result?, initial(3, Some(3), 2));
    assert_eq!(events[0].downloaded, 0);
    assert_eq!(events[0].attempt, 2);
    assert!(!request.to_ascii_lowercase().contains("range:"));
    Ok(())
}

#[test]
fn interrupted_http_transfer_resumes_without_counting_prefix_twice() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let part = temporary.path().join("runtime.part");
    let (failed, first_events, _) = transfer_response(
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc",
        &part,
        1,
    )?;
    assert!(failed.is_err());
    assert_eq!(fs::read(&part)?, b"abc");
    assert_eq!(first_events.last().map(|event| event.downloaded), Some(3));
    let (result, events, request) = transfer_response(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef",
        &part,
        2,
    )?;
    assert_eq!(result?, initial(6, Some(6), 2));
    assert_eq!(events[0].downloaded, 3);
    assert_eq!(events[0].attempt, 2);
    assert!(request.to_ascii_lowercase().contains("range: bytes=3-"));
    assert_eq!(fs::read(part)?, b"abcdef");
    Ok(())
}
