//! Staged shim publication with a recoverable ownership journal and fail-closed old entries.
//! 带可恢复所有权日志的 shim 暂存发布；旧入口默认拒绝误路由。

use crate::{
    commands::WindowsCommandCaseMapper,
    registry::{Location, PublishedShim, Snapshot, Transaction},
    state::{WindowsStateFileSystem, io_error},
    storage::{DirectoryLease, invalid, sha256_file},
};
use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{
        CommandKey, CommandKind, CommandRequest, CommandStatus, RuntimeCommandManifest,
        command_union,
    },
    state::StateFileSystem,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_BINARY: u64 = 128 * 1024 * 1024;
const DISPATCHER: &str = "pyrudder-dispatch.exe";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    location: Location,
    entries: Vec<PublishedShim>,
}

struct Artifact {
    record: PublishedShim,
    source: Source,
}
enum Source {
    File(PathBuf),
    Text(Vec<u8>),
}

fn conflict(message: &str) -> Error {
    Error::new(ErrorKind::Conflict, message)
}

fn equivalent(
    left: &CommandRequest,
    right: &CommandRequest,
    manifests: &[RuntimeCommandManifest],
) -> bool {
    manifests.iter().all(|manifest| {
        manifest.commands().get(left).map(|entry| &entry.winner)
            == manifest.commands().get(right).map(|entry| &entry.winner)
    })
}

fn plan(location: &Location, snapshot: &Snapshot) -> Result<Vec<Artifact>> {
    let mut result: BTreeMap<String, Artifact> = BTreeMap::new();
    let console = location.install_dir.join("pyrudder-shim-console.exe");
    let gui = location.install_dir.join("pyrudder-shim-gui.exe");
    let console_hash = sha256_file(&console, MAX_BINARY)?;
    let gui_hash = sha256_file(&gui, MAX_BINARY)?;
    let mut needs_dispatcher = false;
    for (request, _) in command_union(&snapshot.manifests)? {
        let (name, exact) = match &request {
            CommandRequest::Bare(key) => (key.as_str(), false),
            CommandRequest::Exact(key) => (key.as_str(), true),
        };
        let filename = if exact {
            name.to_owned()
        } else {
            format!("{name}.exe")
        };
        let filename = CommandKey::new(&filename, &WindowsCommandCaseMapper)?
            .as_str()
            .to_owned();
        if let Some(previous) = result.get(&filename) {
            if !equivalent(&previous.record.request()?, &request, &snapshot.manifests) {
                return Err(conflict(
                    "Bare/exact commands require different targets for the same physical shim filename",
                ));
            }
            continue;
        }
        let mut subsystem = None;
        for manifest in &snapshot.manifests {
            if let Some(entry) = manifest.commands().get(&request) {
                if let CommandStatus::Available(kind) = entry.winner.status {
                    let is_gui = kind == CommandKind::PeGui;
                    if subsystem.is_some_and(|previous| previous != is_gui) {
                        return Err(conflict(
                            "Same command requires both Console and GUI shims across runtimes",
                        ));
                    }
                    subsystem = Some(is_gui);
                }
            }
        }
        let is_gui = subsystem.unwrap_or(false);
        let extension = Path::new(&filename)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let (source, digest) = if exact && matches!(extension, "CMD" | "BAT" | "PS1") {
            needs_dispatcher = true;
            let text = if extension == "PS1" {
                format!(
                    "\u{feff}& ($PSScriptRoot + '\\{DISPATCHER}') '__dispatch' '{}' '--' @args\r\nexit $LASTEXITCODE\r\n",
                    filename.replace('\'', "''")
                )
            } else {
                format!(
                    "@echo off\r\n\"%~dp0{DISPATCHER}\" __dispatch \"%~nx0\" -- %*\r\nexit /b %errorlevel%\r\n"
                )
            };
            let bytes = text.into_bytes();
            let digest = format!("{:x}", Sha256::digest(&bytes));
            (Source::Text(bytes), digest)
        } else if is_gui {
            (Source::File(gui.clone()), gui_hash.clone())
        } else {
            (Source::File(console.clone()), console_hash.clone())
        };
        result.insert(
            filename.clone(),
            Artifact {
                record: PublishedShim {
                    filename,
                    request_name: name.to_owned(),
                    exact,
                    gui: is_gui,
                    sha256: digest,
                },
                source,
            },
        );
    }
    if needs_dispatcher {
        let source = location.install_dir.join("pyrudder.exe");
        let digest = sha256_file(&source, MAX_BINARY)?;
        result.insert(
            DISPATCHER.into(),
            Artifact {
                record: PublishedShim {
                    filename: DISPATCHER.into(),
                    request_name: "pyrudder-dispatch".into(),
                    exact: false,
                    gui: false,
                    sha256: digest,
                },
                source: Source::File(source),
            },
        );
    }
    Ok(result.into_values().collect())
}

/// Computes the complete desired index without creating or changing any files.
/// 计算完整期望索引，不创建或修改文件。
///
/// # Errors
/// Rejects physical-name/subsystem conflicts and unavailable templates.
/// 拒绝物理名称/子系统冲突和不可用模板。
pub fn desired_index(location: &Location, snapshot: &Snapshot) -> Result<Vec<PublishedShim>> {
    location.validate()?;
    Ok(plan(location, snapshot)?
        .into_iter()
        .map(|artifact| artifact.record)
        .collect())
}

/// Publishes physical entries before committing one coherent routing snapshot.
/// 先发布物理入口，再提交单一一致路由快照。
///
/// Uncommitted new shims have no routing identity and cannot select an arbitrary runtime.
/// Stale files remain denied by the new index and are retained in pending cleanup on failure.
/// 未提交的新 shim 没有路由身份，不能选择任意运行时；新索引拒绝陈旧文件，
/// 清理失败时将其保留在待清理记录中。
///
/// # Errors
/// Rejects foreign files, changed ownership, concurrent writers, or failed publication.
/// 拒绝外部文件、变更的所有权、并发写入或发布失败。
pub fn publish(location: &Location, mut transaction: Transaction) -> Result<u64> {
    location.validate()?;
    transaction.snapshot.validate()?;
    let _directory = DirectoryLease::acquire(&location.shims_dir, true)?;
    let _programs = DirectoryLease::acquire(&location.install_dir, false)?;
    location.publish(&location.shims_dir)?;
    let artifacts = plan(location, &transaction.snapshot)?;
    let journal_path = location.shims_dir.join("pyrudder-publication.json");
    let mut owned = transaction.snapshot.shims.clone();
    owned.extend(transaction.snapshot.pending_cleanup.clone());
    if let Some(bytes) = WindowsStateFileSystem.read_file(&journal_path, 32 * 1024 * 1024)? {
        let journal: Journal = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Invalid publication recovery journal"))?;
        if journal.schema_version != 1
            || journal.location != *location
            || journal.entries.len() > 131_072
        {
            return Err(invalid("Mismatched publication journal"));
        }
        for entry in &journal.entries {
            entry.validate()?;
        }
        owned.extend(journal.entries);
    }
    // Check the entire destination set before writing a journal or changing an existing entry.
    // 写日志或改变已有入口前，先检查整个目标集合。
    for artifact in &artifacts {
        let destination = location.shims_dir.join(&artifact.record.filename);
        if let Some(bytes) = WindowsStateFileSystem.read_file(&destination, 128 * 1024 * 1024)? {
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if !owned.iter().any(|entry| {
                entry
                    .filename
                    .eq_ignore_ascii_case(&artifact.record.filename)
                    && entry.sha256 == digest
            }) {
                return Err(conflict(
                    "Shim destination contains a foreign or modified file; it was not overwritten",
                ));
            }
        }
    }
    owned.extend(artifacts.iter().map(|artifact| artifact.record.clone()));
    let journal = Journal {
        schema_version: 1,
        location: location.clone(),
        entries: owned.clone(),
    };
    let bytes =
        serde_json::to_vec(&journal).map_err(|_| invalid("Cannot encode publication journal"))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(invalid("Publication journal exceeds its limit"));
    }
    WindowsStateFileSystem.write_atomic(&journal_path, &bytes)?;
    for artifact in &artifacts {
        install_artifact(artifact, &location.shims_dir)?;
    }
    let expected: Vec<_> = artifacts
        .into_iter()
        .map(|artifact| artifact.record)
        .collect();
    let mut stale = BTreeMap::new();
    for entry in owned {
        if !expected
            .iter()
            .any(|current| current.filename.eq_ignore_ascii_case(&entry.filename))
        {
            stale.insert(entry.filename.clone(), entry);
        }
    }
    transaction.snapshot.shims = expected;
    transaction.snapshot.pending_cleanup = stale.into_values().collect();
    // Persist stale ownership first; a crash during cleanup must not lose recovery evidence.
    // 先持久化陈旧入口所有权，避免清理期间崩溃丢失恢复依据。
    let generation = transaction.commit()?;
    if let Err(error) = finish_cleanup(location, &journal_path) {
        eprintln!(
            "PyRudder: routing generation {generation} committed; cleanup deferred: {error} / 路由已提交，清理延后；请稍后运行 rehash。"
        );
    }
    Ok(generation)
}

fn finish_cleanup(location: &Location, journal_path: &Path) -> Result<()> {
    let registry = crate::registry::Registry::new(&location.config_dir)?;
    if let Ok(mut cleanup) = registry.transaction() {
        cleanup.snapshot.pending_cleanup.retain(|entry| {
            let path = location.shims_dir.join(&entry.filename);
            match sha256_file(&path, MAX_BINARY) {
                Ok(digest) if digest == entry.sha256 => fs::remove_file(&path).is_err(),
                Err(error) if error.kind() == ErrorKind::NotInstalled => false,
                _ => true,
            }
        });
        let remaining = cleanup
            .snapshot
            .shims
            .iter()
            .chain(&cleanup.snapshot.pending_cleanup)
            .cloned()
            .collect();
        let journal = Journal {
            schema_version: 1,
            location: location.clone(),
            entries: remaining,
        };
        let bytes =
            serde_json::to_vec(&journal).map_err(|_| invalid("Cannot encode cleanup journal"))?;
        WindowsStateFileSystem.write_atomic(journal_path, &bytes)?;
        cleanup.commit()?;
    }
    Ok(())
}

fn install_artifact(artifact: &Artifact, directory: &Path) -> Result<()> {
    let destination = directory.join(&artifact.record.filename);
    if sha256_file(&destination, MAX_BINARY).is_ok_and(|digest| digest == artifact.record.sha256) {
        return Ok(());
    }
    if !destination.exists() {
        if let Source::File(source) = &artifact.source {
            if fs::hard_link(source, &destination).is_ok() {
                if sha256_file(&destination, MAX_BINARY)? != artifact.record.sha256 {
                    return Err(conflict("Template changed during publication"));
                }
                return Ok(());
            }
        }
    }
    let mut staged = tempfile::Builder::new()
        .prefix(".pyrudder-shim-")
        .tempfile_in(directory)
        .map_err(|error| io_error("stage shim", directory, &error))?;
    match &artifact.source {
        Source::Text(bytes) => staged
            .write_all(bytes)
            .map_err(|error| io_error("write wrapper", &destination, &error))?,
        Source::File(source) => {
            let mut source_file = File::open(source)
                .map_err(|error| io_error("read shim template", source, &error))?
                .take(MAX_BINARY + 1);
            std::io::copy(&mut source_file, &mut staged)
                .map_err(|error| io_error("copy shim template", source, &error))?;
        }
    }
    staged
        .as_file()
        .sync_all()
        .map_err(|error| io_error("flush shim", &destination, &error))?;
    let staged = staged.into_temp_path();
    if sha256_file(&staged, MAX_BINARY)? != artifact.record.sha256 {
        return Err(conflict("Staged shim differs from its planned digest"));
    }
    staged
        .persist(&destination)
        .map_err(|error| io_error("publish shim", &destination, &error.error))?;
    Ok(())
}
