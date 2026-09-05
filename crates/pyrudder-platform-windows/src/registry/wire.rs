//! Versioned wire records rebuilt through domain constructors, never unchecked private fields.
//! 带版本的传输记录经领域构造器重建，绝不绕过私有字段校验。

use super::{Location, MAX_SNAPSHOT_BYTES, PublishedShim, Snapshot};
use crate::{
    commands::WindowsCommandCaseMapper,
    storage::{invalid, path_key},
};
use pyrudder_core::{
    Result,
    commands::{
        CommandExtension, CommandKind, CommandOrigin, CommandRejection, CommandStatus,
        CommandTarget, DirectoryFingerprint, EntryFingerprint, RuntimeCommandManifest,
    },
    runtime::{RuntimeHealth, RuntimeOrigin, RuntimeRecord},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    time::SystemTime,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    generation: u64,
    runtimes: Vec<RuntimeDocument>,
    manifests: Vec<ManifestDocument>,
    shims: Vec<PublishedShim>,
    pending_cleanup: Vec<PublishedShim>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeDocument {
    id: String,
    root: PathBuf,
    scripts: Vec<PathBuf>,
    origin: Origin,
    health: String,
    reason: Option<String>,
    aliases: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Origin {
    External {
        executable: PathBuf,
    },
    Managed {
        provider: String,
        artifact_url: String,
        artifact_sha256: [u8; 32],
        ownership_id: String,
    },
}

impl RuntimeDocument {
    fn from_record(record: &RuntimeRecord) -> Self {
        let origin = match record.origin() {
            RuntimeOrigin::External {
                registered_executable,
            } => Origin::External {
                executable: registered_executable.clone(),
            },
            RuntimeOrigin::Managed {
                provider,
                artifact_url,
                artifact_sha256,
                ownership_id,
            } => Origin::Managed {
                provider: provider.clone(),
                artifact_url: artifact_url.clone(),
                artifact_sha256: *artifact_sha256,
                ownership_id: ownership_id.to_string(),
            },
        };
        let (health, reason) = match record.health() {
            RuntimeHealth::Ready => ("ready", None),
            RuntimeHealth::Unchecked => ("unchecked", None),
            RuntimeHealth::PendingRemoval => ("pending_removal", None),
            RuntimeHealth::Broken { reason } => ("broken", Some(reason.clone())),
            RuntimeHealth::Unavailable { reason } => ("unavailable", Some(reason.clone())),
        };
        Self {
            id: record.id().to_string(),
            root: record.root().to_path_buf(),
            scripts: record.command_dirs().to_vec(),
            origin,
            health: health.into(),
            reason,
            aliases: record.aliases().iter().map(ToString::to_string).collect(),
        }
    }

    fn into_record(self) -> Result<RuntimeRecord> {
        let origin = match self.origin {
            Origin::External { executable } => {
                path_key(&executable)?;
                RuntimeOrigin::External {
                    registered_executable: executable,
                }
            }
            Origin::Managed {
                provider,
                artifact_url,
                artifact_sha256,
                ownership_id,
            } => RuntimeOrigin::Managed {
                provider,
                artifact_url,
                artifact_sha256,
                ownership_id: ownership_id.parse()?,
            },
        };
        let mut record = RuntimeRecord::new(self.id.parse()?, self.root, self.scripts, origin)?;
        let health = match self.health.as_str() {
            "ready" => RuntimeHealth::Ready,
            "unchecked" => RuntimeHealth::Unchecked,
            "pending_removal" => RuntimeHealth::PendingRemoval,
            "broken" => RuntimeHealth::Broken {
                reason: self
                    .reason
                    .ok_or_else(|| invalid("Missing health reason"))?,
            },
            "unavailable" => RuntimeHealth::Unavailable {
                reason: self
                    .reason
                    .ok_or_else(|| invalid("Missing health reason"))?,
            },
            _ => return Err(invalid("Unknown persisted runtime health")),
        };
        record.set_health(health);
        if self.aliases.len() > 64 {
            return Err(invalid("Too many runtime aliases"));
        }
        for alias in self.aliases {
            if !record.add_alias(alias.parse()?) {
                return Err(invalid("Duplicate alias"));
            }
        }
        Ok(record)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    name: Vec<u16>,
    size: u64,
    modified: SystemTime,
    attributes: u32,
}

impl Fingerprint {
    fn from_entry(entry: &EntryFingerprint) -> Self {
        Self {
            name: entry.name.encode_wide().collect(),
            size: entry.size,
            modified: entry.modified,
            attributes: entry.attributes,
        }
    }
    fn into_entry(self) -> Result<EntryFingerprint> {
        if self.name.is_empty() || self.name.len() > 255 || self.name.contains(&0) {
            return Err(invalid("Invalid fingerprint name"));
        }
        Ok(EntryFingerprint {
            name: OsString::from_wide(&self.name),
            size: self.size,
            modified: self.modified,
            attributes: self.attributes,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Directory {
    path: PathBuf,
    modified: Option<SystemTime>,
    entries: Vec<Fingerprint>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    path: PathBuf,
    script_index: Option<usize>,
    status: String,
    fingerprint: Fingerprint,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    runtime_id: String,
    targets: Vec<Target>,
    directories: Vec<Directory>,
}

fn status_text(status: CommandStatus) -> &'static str {
    match status {
        CommandStatus::Available(CommandKind::PeConsole) => "pe_console",
        CommandStatus::Available(CommandKind::PeGui) => "pe_gui",
        CommandStatus::Available(CommandKind::Cmd) => "cmd",
        CommandStatus::Available(CommandKind::Bat) => "bat",
        CommandStatus::Available(CommandKind::PowerShell) => "powershell",
        CommandStatus::Rejected(CommandRejection::InvalidPe) => "invalid_pe",
        CommandStatus::Rejected(CommandRejection::UnsupportedImage) => "unsupported_image",
        CommandStatus::Rejected(CommandRejection::ReparsePoint) => "reparse_point",
    }
}

fn parse_status(status: &str) -> Result<CommandStatus> {
    Ok(match status {
        "pe_console" => CommandStatus::Available(CommandKind::PeConsole),
        "pe_gui" => CommandStatus::Available(CommandKind::PeGui),
        "cmd" => CommandStatus::Available(CommandKind::Cmd),
        "bat" => CommandStatus::Available(CommandKind::Bat),
        "powershell" => CommandStatus::Available(CommandKind::PowerShell),
        "invalid_pe" => CommandStatus::Rejected(CommandRejection::InvalidPe),
        "unsupported_image" => CommandStatus::Rejected(CommandRejection::UnsupportedImage),
        "reparse_point" => CommandStatus::Rejected(CommandRejection::ReparsePoint),
        _ => return Err(invalid("Unknown persisted command kind")),
    })
}

impl ManifestDocument {
    fn from_manifest(manifest: &RuntimeCommandManifest) -> Self {
        let mut targets = BTreeMap::new();
        for entry in manifest.commands().values() {
            for target in std::iter::once(&entry.winner).chain(&entry.shadowed) {
                targets
                    .entry((target.origin, target.path.clone()))
                    .or_insert_with(|| Target {
                        path: target.path.clone(),
                        script_index: match target.origin {
                            CommandOrigin::Root => None,
                            CommandOrigin::Script(index) => Some(index),
                        },
                        status: status_text(target.status).into(),
                        fingerprint: Fingerprint::from_entry(&target.fingerprint),
                    });
            }
        }
        Self {
            runtime_id: manifest.runtime_id().to_string(),
            targets: targets.into_values().collect(),
            directories: manifest
                .directories()
                .iter()
                .map(|directory| Directory {
                    path: directory.path.clone(),
                    modified: directory.modified,
                    entries: directory
                        .entries
                        .iter()
                        .map(Fingerprint::from_entry)
                        .collect(),
                })
                .collect(),
        }
    }

    fn into_manifest(self, runtimes: &[RuntimeRecord]) -> Result<RuntimeCommandManifest> {
        let id = self.runtime_id.parse()?;
        let runtime = runtimes
            .iter()
            .find(|runtime| runtime.id() == &id)
            .ok_or_else(|| invalid("Manifest has no matching runtime"))?;
        if self.targets.len() > 16_384 || self.directories.len() > 64 {
            return Err(invalid("Manifest exceeds entry limits"));
        }
        let mut targets = Vec::new();
        for target in self.targets {
            let parent = target
                .path
                .parent()
                .ok_or_else(|| invalid("Command has no parent"))?;
            let allowed = if let Some(index) = target.script_index {
                runtime
                    .command_dirs()
                    .get(index)
                    .ok_or_else(|| invalid("Invalid command directory index"))?
                    .as_path()
            } else {
                runtime.root()
            };
            if path_key(parent)? != path_key(allowed)? {
                return Err(invalid("Persisted command escapes its source directory"));
            }
            let name = target
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid("Invalid target name"))?;
            let (_, extension) = CommandExtension::split(name)
                .ok_or_else(|| invalid("Unsupported target extension"))?;
            targets.push(CommandTarget {
                path: target.path,
                origin: target
                    .script_index
                    .map_or(CommandOrigin::Root, CommandOrigin::Script),
                extension,
                status: parse_status(&target.status)?,
                fingerprint: target.fingerprint.into_entry()?,
            });
        }
        let mut directories = Vec::new();
        let mut total = 0;
        for directory in self.directories {
            total += directory.entries.len();
            if total > 16_384 {
                return Err(invalid("Too many directory observations"));
            }
            let key = path_key(&directory.path)?;
            if !std::iter::once(runtime.root())
                .chain(runtime.command_dirs().iter().map(PathBuf::as_path))
                .map(path_key)
                .collect::<Result<Vec<_>>>()?
                .contains(&key)
            {
                return Err(invalid("Unbound manifest directory"));
            }
            directories.push(DirectoryFingerprint {
                path: directory.path,
                modified: directory.modified,
                entries: directory
                    .entries
                    .into_iter()
                    .map(Fingerprint::into_entry)
                    .collect::<Result<_>>()?,
            });
        }
        RuntimeCommandManifest::build(
            id,
            targets,
            directories,
            Vec::new(),
            &WindowsCommandCaseMapper,
        )
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<Snapshot> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(invalid("Snapshot exceeds its size limit"));
    }
    let document: Document = serde_json::from_slice(bytes).map_err(|_| {
        invalid("Invalid registry JSON; preserve the file and restore a known good snapshot")
    })?;
    if document.schema_version != 1 {
        return Err(invalid("Unsupported registry schema_version"));
    }
    let runtimes = document
        .runtimes
        .into_iter()
        .map(RuntimeDocument::into_record)
        .collect::<Result<Vec<_>>>()?;
    let manifests = document
        .manifests
        .into_iter()
        .map(|manifest| manifest.into_manifest(&runtimes))
        .collect::<Result<_>>()?;
    let snapshot = Snapshot {
        generation: document.generation,
        runtimes,
        manifests,
        shims: document.shims,
        pending_cleanup: document.pending_cleanup,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

pub(super) fn encode(snapshot: &Snapshot) -> Result<Vec<u8>> {
    snapshot.validate()?;
    let document = Document {
        schema_version: 1,
        generation: snapshot.generation,
        runtimes: snapshot
            .runtimes
            .iter()
            .map(RuntimeDocument::from_record)
            .collect(),
        manifests: snapshot
            .manifests
            .iter()
            .map(ManifestDocument::from_manifest)
            .collect(),
        shims: snapshot.shims.clone(),
        pending_cleanup: snapshot.pending_cleanup.clone(),
    };
    let bytes =
        serde_json::to_vec(&document).map_err(|_| invalid("Cannot encode registry snapshot"))?;
    // Round-trip validation also applies constructor and source-boundary checks to new mutations.
    // 往返校验也将构造器和来源边界检查应用到新修改的数据。
    decode(&bytes)?;
    Ok(bytes)
}

/// Decode location separately from mutable snapshot state. / 将位置契约与可变快照分开解码。
pub(crate) fn decode_location(bytes: &[u8]) -> Result<Location> {
    let location: Location =
        serde_json::from_slice(bytes).map_err(|_| invalid("Invalid shim location file"))?;
    location.validate()?;
    Ok(location)
}
