//! Official runtime lifecycle with durable pending state and explicit owned deletion.
//! 官方运行时生命周期，包含持久待完成状态和显式所有权删除。

use super::{App, find_any, runtime_json, usage};
use crate::arguments::CacheAction;
use pyrudder_core::{
    Error, ErrorKind, Result,
    runtime::{RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord},
    state::StateFileSystem,
};
use pyrudder_platform_windows::{
    commands::discover_commands,
    probe, publication,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, new_identity, path_key},
};
use pyrudder_provider_pythonorg::{PythonOrgProvider, Release};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const SENTINEL: &str = "pyrudder-owner.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    runtime_id: String,
    ownership_id: String,
    root: std::path::PathBuf,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallJournal {
    owner: Owner,
    staging: std::path::PathBuf,
}

impl App {
    pub(super) fn recover(&self, discard: Option<&str>, yes: bool) -> Result<Value> {
        let snapshot = self.registry.load()?;
        let _config = DirectoryLease::acquire(&self.location.config_dir, false)?;
        if let Some(filename) = discard {
            if !yes
                || !filename.starts_with("install-")
                || Path::new(filename).components().count() != 1
                || Path::new(filename)
                    .extension()
                    .is_none_or(|extension| extension != "json")
            {
                return Err(usage(
                    "Use recover --discard-staging <journal filename from recover> --yes",
                ));
            }
            let _installation = FileLease::acquire(
                &self.location.config_dir.join("managed-install.lock"),
                true,
                true,
            )?;
            let snapshot = self.registry.load()?;
            let bytes = WindowsStateFileSystem
                .read_file(&self.location.config_dir.join(filename), 65_536)?
                .ok_or_else(|| usage("Recovery journal missing"))?;
            let journal: InstallJournal =
                serde_json::from_slice(&bytes).map_err(|_| usage("Invalid recovery journal"))?;
            let id: RuntimeId = journal.owner.runtime_id.parse()?;
            let record = snapshot.runtimes.iter().find(|record| record.id() == &id).ok_or_else(|| usage("Recovery requires its owning runtime registration; never infer ownership of an orphan folder"))?;
            let RuntimeOrigin::Managed {
                ownership_id,
                artifact_sha256,
                ..
            } = record.origin()
            else {
                return Err(usage("Staging recovery cannot affect external runtimes"));
            };
            if journal.owner.schema_version != 1
                || journal.owner.ownership_id != ownership_id.to_string()
                || digest_bytes(&journal.owner.sha256)? != *artifact_sha256
                || path_key(record.root())? != path_key(&journal.owner.root)?
            {
                return Err(usage(
                    "Recovery journal does not match its registered owner",
                ));
            }
            let root = self.registered_managed_root(record)?;
            let parent = root
                .parent()
                .ok_or_else(|| install_error("Managed runtime requires a parent directory"))?;
            validate_staging_target(filename, &journal, parent)?;
            let _parent = DirectoryLease::acquire(parent, false)?;
            if WindowsStateFileSystem
                .read_file(&journal.staging.join(SENTINEL), 65_536)?
                .is_some()
            {
                validate_owner(&journal.staging, &journal.owner)?;
            }
            inspect_tree(&journal.staging, 0, &mut 0)?;
            remove_owned_tree(&journal.staging, 0, None)?;
            return Ok(
                json!({"discarded_staging": journal.staging, "journal_preserved": filename, "recoverable": false}),
            );
        }
        let mut journals = Vec::new();
        for (index, entry) in fs::read_dir(&self.location.config_dir)
            .map_err(|_| usage("Cannot enumerate recovery journals"))?
            .enumerate()
        {
            if index > 10_000 {
                return Err(usage("Recovery inventory limit exceeded"));
            }
            let entry = entry.map_err(|_| usage("Cannot inspect recovery journal"))?;
            let filename = entry.file_name().to_string_lossy().into_owned();
            if !filename.starts_with("install-")
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let bytes = WindowsStateFileSystem
                .read_file(&entry.path(), 65_536)?
                .ok_or_else(|| usage("Recovery journal disappeared"))?;
            let journal: InstallJournal =
                serde_json::from_slice(&bytes).map_err(|_| usage("Invalid recovery journal"))?;
            journals.push(json!({"journal": filename, "runtime_id": journal.owner.runtime_id, "staging": journal.staging, "staging_exists": journal.staging.try_exists().ok(), "destination": journal.owner.root}));
        }
        Ok(
            json!({"writes": false, "pending_runtimes": snapshot.runtimes.iter().filter(|record| !matches!(record.health(), RuntimeHealth::Ready)).map(runtime_json).collect::<Vec<_>>(), "pending_shims": snapshot.pending_cleanup, "installation_journals": journals,
            "next": "Retry install <exact version> for Unchecked; retry uninstall <id> --yes for PendingRemoval; run rehash for stale shims. Discard only a named staging journal with --discard-staging <filename> --yes."}),
        )
    }

    pub(super) fn releases(&self, offline: bool) -> Result<Vec<Release>> {
        if self.progress {
            eprintln!("Verifying official Python index... / 正在验证 Python 官方索引……");
        }
        PythonOrgProvider::releases(&self.configuration.paths.cache_dir, offline)
    }

    pub(super) fn download(&self, version: &str, offline: bool) -> Result<Value> {
        let release = PythonOrgProvider::select(&self.releases(offline)?, version)?;
        let path = self.download_archive(&release, offline)?;
        Ok(json!({"release": release, "archive": path, "installed": false}))
    }

    pub(super) fn install(
        &self,
        version: &str,
        alias: Option<&str>,
        offline: bool,
    ) -> Result<Value> {
        let release = PythonOrgProvider::select(&self.releases(offline)?, version)?;
        self.install_release(&release, alias, offline, None)
    }

    pub(super) fn install_release(
        &self,
        release: &Release,
        alias: Option<&str>,
        offline: bool,
        directory: Option<&Path>,
    ) -> Result<Value> {
        let alias = alias
            .map(str::parse::<pyrudder_core::runtime::RuntimeAlias>)
            .transpose()?;
        let id = RuntimeId::new(release.version.parse()?)?;
        let _installation = FileLease::acquire(
            &self.location.config_dir.join("managed-install.lock"),
            true,
            true,
        )?;
        let _runtime = FileLease::acquire(&self.registry.lease_path(&id), true, true)?;
        let mut transaction = self.registry.transaction()?;
        if let Some(existing) = transaction
            .snapshot
            .runtimes
            .iter()
            .find(|record| record.id() == &id)
        {
            if matches!(existing.health(), RuntimeHealth::Ready) {
                return Ok(
                    json!({"already_installed": runtime_json(existing), "hint": "Existing runtime and aliases were preserved."}),
                );
            }
            if matches!(existing.health(), RuntimeHealth::PendingRemoval) {
                return Err(usage(
                    "Finish uninstalling the pending runtime before reinstalling",
                ));
            }
        }
        let root = self.installation_root(&transaction.snapshot.runtimes, &id, directory)?;
        let parent = root
            .parent()
            .ok_or_else(|| install_error("Managed runtime requires a parent directory"))?;
        let _runtimes_parent = DirectoryLease::acquire(parent, true)?;
        let mut record = if let Some(existing) = transaction
            .snapshot
            .runtimes
            .iter()
            .find(|record| record.id() == &id)
        {
            existing.clone()
        } else {
            if root
                .try_exists()
                .map_err(|_| install_error("Cannot inspect managed destination"))?
            {
                return Err(install_error(
                    "Managed destination already exists without an owned registration; it will not be overwritten",
                ));
            }
            let record = RuntimeRecord::new(
                id.clone(),
                root.clone(),
                vec![root.join("Scripts")],
                RuntimeOrigin::Managed {
                    provider: "python.org".into(),
                    artifact_url: release.url.clone(),
                    artifact_sha256: digest_bytes(&release.sha256)?,
                    ownership_id: new_identity()?,
                },
            )?;
            transaction.snapshot.runtimes.push(record.clone());
            transaction.commit()?;
            transaction = self.registry.transaction()?;
            record
        };
        drop(transaction);
        let owner = installation_owner(&record, release, &root)?;
        self.stage_release(release, &owner, offline, parent)?;
        if self.progress {
            eprintln!(
                "Checking Python and initializing bundled pip... / 正在检查 Python 并初始化内置 pip……"
            );
        }
        let _root = DirectoryLease::acquire(&root, false)?;
        let identity = probe::inspect(&root.join("python.exe"), false)?;
        validate_identity(&identity, release, &root)?;
        bootstrap_pip(&root)?;
        let identity = probe::inspect(&root.join("python.exe"), false)?;
        validate_identity(&identity, release, &root)?;
        if let Some(alias) = alias {
            record.add_alias(alias);
        }
        record.set_health(RuntimeHealth::Ready);
        let manifest = discover_commands(&record)?;
        let mut transaction = self.registry.transaction()?;
        transaction
            .snapshot
            .runtimes
            .retain(|existing| existing.id() != &id);
        transaction
            .snapshot
            .manifests
            .retain(|existing| existing.runtime_id() != &id);
        transaction.snapshot.runtimes.push(record.clone());
        transaction.snapshot.manifests.push(manifest);
        let generation = publication::publish(&self.location, transaction)?;
        Ok(
            json!({"installed": runtime_json(&record), "generation": generation, "hint": "Select with pyrudder global <version> or pyrudder local <version>. Installation does not change the active version."}),
        )
    }

    fn stage_release(
        &self,
        release: &Release,
        owner: &Owner,
        offline: bool,
        parent: &Path,
    ) -> Result<()> {
        let root = &owner.root;
        if root
            .try_exists()
            .map_err(|_| install_error("Cannot inspect pending runtime"))?
        {
            return validate_owner(root, owner);
        }
        if self.progress {
            eprintln!(
                "Downloading/verifying Python {}... / 正在下载并验证 Python {}……",
                release.version, release.version
            );
        }
        let archive = self.download_archive(release, offline)?;
        // Keep interrupted staging for inspection; never recursively auto-delete an unverified tree.
        // 保留中断的 staging 供检查；绝不自动递归删除未验证的目录树。
        let attempt = new_identity()?;
        let staging = parent.join(format!(".pyrudder-staging-{attempt}"));
        fs::create_dir(&staging)
            .map_err(|_| install_error("Cannot create unique same-volume staging"))?;
        // Unique journals preserve every interrupted attempt rather than losing prior ownership.
        // 唯一日志保留每次中断尝试，避免丢失此前的所有权记录。
        let journal = self
            .location
            .config_dir
            .join(format!("install-{}-{attempt}.json", owner.runtime_id));
        let bytes = serde_json::to_vec(&json!({"owner": owner, "staging": staging}))
            .map_err(|_| install_error("Cannot encode install recovery journal"))?;
        WindowsStateFileSystem.write_atomic(&journal, &bytes)?;
        if self.progress {
            eprintln!("Extracting authenticated runtime... / 正在解压已认证运行时……");
        }
        pyrudder_provider_pythonorg::extract(&archive, &release.sha256, &staging).map_err(|error| error.with_hint(format!("Staging retained at {}. Retry install after resolving the error; no unchecked runtime can be selected.", staging.display())))?;
        let bytes =
            serde_json::to_vec(owner).map_err(|_| install_error("Cannot encode runtime owner"))?;
        WindowsStateFileSystem.write_atomic(&staging.join(SENTINEL), &bytes)?;
        fs::rename(&staging, root).map_err(|_| {
            install_error(
                "Cannot atomically publish staged runtime; install journal retains its location",
            )
        })
    }

    pub(super) fn uninstall(&self, version: &str, force: bool, yes: bool) -> Result<Value> {
        if !yes {
            return Err(usage(
                "uninstall deletes an owned managed Python, including packages; pass --yes to confirm",
            ));
        }
        let _installation = FileLease::acquire(
            &self.location.config_dir.join("managed-install.lock"),
            true,
            true,
        )?;
        let mut transaction = self.registry.transaction()?;
        let mut record = find_any(&transaction.snapshot, version)?.clone();
        let RuntimeOrigin::Managed {
            ownership_id,
            artifact_sha256,
            ..
        } = record.origin()
        else {
            return Err(usage(
                "External runtimes cannot be deleted; use unregister instead",
            ));
        };
        let expected_root = self.registered_managed_root(&record)?;
        let owner = Owner {
            schema_version: 1,
            runtime_id: record.id().to_string(),
            ownership_id: ownership_id.to_string(),
            root: expected_root.clone(),
            sha256: artifact_sha256
                .iter()
                .fold(String::with_capacity(64), |mut text, byte| {
                    use std::fmt::Write;
                    let _written = write!(text, "{byte:02x}");
                    text
                }),
        };
        if !force {
            self.check_active(record.id())?;
        }
        let _runtime = FileLease::acquire(&self.registry.lease_path(record.id()), true, false)?;
        let _parent = DirectoryLease::acquire(
            expected_root
                .parent()
                .ok_or_else(|| install_error("Managed runtime requires a parent directory"))?,
            false,
        )?;
        let present = expected_root
            .try_exists()
            .map_err(|_| install_error("Cannot inspect uninstall root"))?;
        if present {
            validate_owner(&expected_root, &owner)?;
            inspect_tree(&expected_root, 0, &mut 0)?;
        }
        record.set_health(RuntimeHealth::PendingRemoval);
        transaction
            .snapshot
            .runtimes
            .retain(|candidate| candidate.id() != record.id());
        transaction.snapshot.runtimes.push(record.clone());
        transaction
            .snapshot
            .manifests
            .retain(|manifest| manifest.runtime_id() != record.id());
        publication::publish(&self.location, transaction)?;
        if present {
            remove_owned_tree(&expected_root, 0, Some(&owner))?;
        }
        let mut transaction = self.registry.transaction()?;
        transaction
            .snapshot
            .runtimes
            .retain(|candidate| candidate.id() != record.id());
        publication::publish(&self.location, transaction)?;
        Ok(
            json!({"uninstalled": record.id().to_string(), "deleted_owned_root": expected_root, "recoverable": false, "selection_files_preserved": true}),
        )
    }

    pub(super) fn cache(&self, action: &CacheAction) -> Result<Value> {
        let directory = &self.configuration.paths.downloads_dir;
        match action {
            CacheAction::List => {
                let mut entries = Vec::new();
                if directory
                    .try_exists()
                    .map_err(|_| usage("Cannot inspect cache"))?
                {
                    let _anchor = DirectoryLease::acquire(directory, false)?;
                    for (index, item) in fs::read_dir(directory)
                        .map_err(|_| usage("Cannot read cache"))?
                        .enumerate()
                    {
                        if index > 10_000 {
                            return Err(usage("Cache inventory limit exceeded"));
                        }
                        let item = item.map_err(|_| usage("Cannot read cache entry"))?;
                        let name = item.file_name().to_string_lossy().into_owned();
                        if cache_name(&name) {
                            entries.push(json!({"filename": name, "path": item.path(), "size": item.metadata().ok().map(|metadata| metadata.len())}));
                        }
                    }
                }
                Ok(
                    json!({"downloads": directory, "signed_index_cache": self.configuration.paths.cache_dir, "entries": entries}),
                )
            }
            CacheAction::Remove { filename, yes } => {
                if !yes || !cache_name(filename) {
                    return Err(usage(
                        "Use cache remove <64-hex-digest.zip|part|etag> --yes; only one download-cache file is removed",
                    ));
                }
                let _directory = DirectoryLease::acquire(directory, false)?;
                let _lock =
                    FileLease::acquire(&directory.join("pythonorg-download.lock"), true, true)?;
                let target = directory.join(filename);
                let metadata = fs::symlink_metadata(&target)
                    .map_err(|_| usage("Cannot inspect named cache entry"))?;
                if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                    return Err(usage("Cache deletion requires an ordinary file"));
                }
                fs::remove_file(&target)
                    .map_err(|_| usage("Cannot delete named cache entry; it may be in use"))?;
                Ok(json!({"deleted": target, "recoverable": false, "runtime_files_changed": false}))
            }
        }
    }

    pub(super) fn managed_install_directory(&self, directory: &Path) -> Result<PathBuf> {
        let directory = WindowsStateFileSystem.normalize_directory(directory)?;
        let paths = &self.configuration.paths;
        for protected in [
            &self.location.install_dir,
            &self.location.config_dir,
            &self.location.shims_dir,
            &paths.downloads_dir,
            &paths.cache_dir,
            &paths.temp_dir,
        ] {
            if paths_overlap(&directory, protected)? {
                return Err(install_error(
                    "Python installation directory must not overlap PyRudder's bin, config, shims, downloads, cache or temp directories",
                ));
            }
        }
        Ok(directory)
    }

    fn installation_root(
        &self,
        records: &[RuntimeRecord],
        id: &RuntimeId,
        directory: Option<&Path>,
    ) -> Result<PathBuf> {
        // Pending installations retain their recorded location even after defaults change.
        // 即使默认目录已改变，待完成安装也始终使用原登记位置。
        if let Some(existing) = records.iter().find(|record| record.id() == id) {
            return self.registered_managed_root(existing);
        }
        let directory = self.managed_install_directory(
            directory.unwrap_or(&self.configuration.paths.runtimes_dir),
        )?;
        let root = directory.join(id.to_string());
        for existing in records {
            if path_key(&directory)?.starts_with(path_key(existing.root())?)
                || paths_overlap(&root, existing.root())?
            {
                return Err(install_error(
                    "Installation directory must not overlap an existing Python runtime",
                ));
            }
        }
        Ok(root)
    }

    fn registered_managed_root(&self, record: &RuntimeRecord) -> Result<PathBuf> {
        let RuntimeOrigin::Managed {
            provider,
            artifact_url,
            ..
        } = record.origin()
        else {
            return Err(install_error(
                "Only registered managed runtimes have owned directories",
            ));
        };
        let root = WindowsStateFileSystem.normalize_directory(record.root())?;
        if provider != "python.org"
            || !artifact_url.starts_with("https://www.python.org/ftp/python/")
            || record.id().installation().is_some()
            || root
                .file_name()
                .is_none_or(|name| !name.eq_ignore_ascii_case(record.id().to_string()))
        {
            return Err(install_error(
                "Managed runtime provenance and exact runtime-ID directory must match its registration",
            ));
        }
        let parent = root
            .parent()
            .ok_or_else(|| install_error("Managed runtime requires a parent directory"))?;
        self.managed_install_directory(parent)?;
        Ok(root)
    }
}

fn paths_overlap(left: &Path, right: &Path) -> Result<bool> {
    let left = path_key(left)?;
    let right = path_key(right)?;
    Ok(left.starts_with(&right) || right.starts_with(&left))
}

fn validate_staging_target(filename: &str, journal: &InstallJournal, parent: &Path) -> Result<()> {
    let prefix = format!("install-{}-", journal.owner.runtime_id);
    let attempt = filename
        .strip_prefix(&prefix)
        .and_then(|name| name.strip_suffix(".json"))
        .ok_or_else(|| usage("Recovery journal filename does not match its runtime owner"))?
        .parse::<pyrudder_core::runtime::InstallationId>()?;
    // The journal and staging share one random attempt identity, not only a name prefix.
    // 日志与 staging 共享同一个随机尝试标识，不能只凭文件名前缀授权删除。
    let expected = parent.join(format!(".pyrudder-staging-{attempt}"));
    if path_key(&journal.staging)? != path_key(&expected)? {
        return Err(usage(
            "Recovery deletion requires the exact staging directory bound to this journal; older unbound journals require manual inspection",
        ));
    }
    Ok(())
}

fn cache_name(name: &str) -> bool {
    name.split_once('.').is_some_and(|(hash, extension)| {
        hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            && matches!(extension, "zip" | "part" | "etag")
    })
}

fn digest_bytes(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.is_ascii() {
        return Err(usage("Invalid archive digest"));
    }
    let mut digest = [0u8; 32];
    for (index, item) in digest.iter_mut().enumerate() {
        *item = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| usage("Invalid digest"))?;
    }
    Ok(digest)
}

fn validate_identity(
    identity: &probe::PythonIdentity,
    release: &Release,
    root: &Path,
) -> Result<()> {
    if identity.version != release.version
        || path_key(&identity.prefix)? != path_key(root)?
        || identity.scripts.len() != 1
        || path_key(&identity.scripts[0])? != path_key(&root.join("Scripts"))?
    {
        return Err(install_error(
            "Installed Python identity differs from the signed artifact/managed root",
        ));
    }
    Ok(())
}

fn validate_owner(root: &Path, expected: &Owner) -> Result<()> {
    let _root = DirectoryLease::acquire(root, false)?;
    let bytes = WindowsStateFileSystem
        .read_file(&root.join(SENTINEL), 65_536)?
        .ok_or_else(|| {
            install_error("Missing managed ownership sentinel; deletion/recovery refused")
        })?;
    let actual: Owner = serde_json::from_slice(&bytes)
        .map_err(|_| install_error("Invalid managed ownership sentinel"))?;
    if actual.schema_version != 1
        || actual.runtime_id != expected.runtime_id
        || actual.ownership_id != expected.ownership_id
        || actual.sha256 != expected.sha256
        || path_key(&actual.root)? != path_key(&expected.root)?
    {
        return Err(install_error(
            "Managed ownership sentinel mismatch; deletion/recovery refused",
        ));
    }
    Ok(())
}

// Official ZIPs can already contain pip's module without any Scripts launchers.
// ensurepip --upgrade then does nothing; reinstall its authenticated bundled wheel offline.
// 官方 ZIP 可能已包含 pip 模块，却没有任何 Scripts 启动器。
// 此时 ensurepip --upgrade 不会执行安装；必须离线重装已认证的内置 wheel。
const BOOTSTRAP_PIP_SCRIPT: &str = r"
import ensurepip
import os
import pathlib
import runpy
import subprocess
import sys

# Ignore caller configuration that could redirect installation or suppress launchers.
# 忽略可能改变安装目标或禁止生成启动器的调用方配置。
for key in tuple(os.environ):
    if key.upper().startswith('PIP_') or key.upper() == 'ENSUREPIP_OPTIONS':
        del os.environ[key]
os.environ['PIP_CONFIG_FILE'] = os.devnull
bundled = pathlib.Path(ensurepip.__file__).resolve().parent / '_bundled'
wheels = list(bundled.glob('pip-*-py3-none-any.whl'))
if len(wheels) != 1 or not wheels[0].is_file():
    raise RuntimeError('Expected exactly one bundled pip wheel')

# Execute pip from that wheel even when an interrupted attempt left its module incomplete.
# 即使上次中断留下不完整的 pip 模块，也直接使用该 wheel 中的 pip 执行安装。
sys.path.insert(0, str(wheels[0]))
sys.argv[1:] = [
    '--isolated', '--disable-pip-version-check', 'install', '--no-index',
    '--no-cache-dir', '--no-deps', '--force-reinstall', '--no-warn-script-location',
    '--prefix', sys.prefix, str(wheels[0]),
]
try:
    runpy.run_module('pip', run_name='__main__', alter_sys=True)
except SystemExit as result:
    if result.code not in (None, 0):
        raise

# An install is ready only when all promised pip launchers actually execute successfully.
# 仅在承诺提供的全部 pip 启动器确实执行成功后，才能将安装标记为就绪。
scripts = pathlib.Path(sys.prefix) / 'Scripts'
for name in ('pip.exe', 'pip3.exe', f'pip3.{sys.version_info.minor}.exe'):
    subprocess.run(
        [str(scripts / name), '--version'], check=True, timeout=20,
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
";

fn bootstrap_pip(root: &Path) -> Result<()> {
    let mut child = Command::new(root.join("python.exe"))
        .args(["-I", "-c", BOOTSTRAP_PIP_SCRIPT])
        .current_dir(root)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONSTARTUP")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|_| install_error("Cannot bootstrap bundled pip"))?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                return Err(install_error(
                    "Bundled ensurepip failed; installation remains Unchecked and can be retried",
                ));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _killed = child.kill();
                let _waited = child.wait();
                return Err(install_error(
                    "ensurepip timed out; installation remains Unchecked",
                ));
            }
        }
    }
}

fn inspect_tree(root: &Path, depth: usize, count: &mut usize) -> Result<()> {
    if depth > 64 {
        return Err(install_error("Owned tree exceeds deletion depth limit"));
    }
    let _anchor = DirectoryLease::acquire(root, false)?;
    for item in fs::read_dir(root).map_err(|_| install_error("Cannot inspect owned tree"))? {
        *count += 1;
        if *count > 100_000 {
            return Err(install_error("Owned tree exceeds deletion entry limit"));
        }
        let item = item.map_err(|_| install_error("Cannot inspect owned entry"))?;
        let metadata = fs::symlink_metadata(item.path())
            .map_err(|_| install_error("Cannot inspect owned metadata"))?;
        if metadata.file_attributes() & 0x400 != 0 || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(install_error(
                "Owned tree contains a reparse point or special file; remove it manually before uninstalling",
            ));
        }
        if metadata.is_dir() {
            inspect_tree(&item.path(), depth + 1, count)?;
        }
    }
    Ok(())
}

fn remove_owned_tree(root: &Path, depth: usize, owner: Option<&Owner>) -> Result<()> {
    if depth > 64 {
        return Err(install_error("Owned tree changed during removal"));
    }
    let anchor = DirectoryLease::acquire(root, false)?;
    for (index, item) in fs::read_dir(root)
        .map_err(|_| install_error("Cannot enumerate owned removal tree"))?
        .enumerate()
    {
        if index > 100_000 {
            return Err(install_error("Removal entry limit exceeded"));
        }
        let item = item.map_err(|_| install_error("Cannot read removal entry"))?;
        if owner.is_some() && item.file_name().eq_ignore_ascii_case(SENTINEL) {
            continue;
        }
        let metadata = fs::symlink_metadata(item.path())
            .map_err(|_| install_error("Cannot revalidate removal entry"))?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(install_error(
                "Reparse point appeared during removal; operation stopped",
            ));
        }
        if metadata.is_dir() {
            remove_owned_tree(&item.path(), depth + 1, None)?;
        } else if metadata.is_file() {
            fs::remove_file(item.path()).map_err(|_| {
                install_error(
                    "Owned file is busy or cannot be deleted; rerun uninstall after closing Python",
                )
            })?;
        } else {
            return Err(install_error("Special file appeared during removal"));
        }
    }
    if owner.is_some() {
        fs::remove_file(root.join(SENTINEL))
            .map_err(|_| install_error("Cannot remove ownership sentinel"))?;
    }
    drop(anchor);
    if fs::remove_dir(root).is_err() {
        if let Some(owner) = owner {
            let _root = DirectoryLease::acquire(root, false)?;
            let bytes = serde_json::to_vec(owner)
                .map_err(|_| install_error("Cannot restore ownership sentinel"))?;
            WindowsStateFileSystem.write_atomic(&root.join(SENTINEL), &bytes)?;
        }
        return Err(install_error(
            "Cannot remove empty runtime directory; pending registration and ownership retained for retry",
        ));
    }
    Ok(())
}

fn installation_owner(record: &RuntimeRecord, release: &Release, root: &Path) -> Result<Owner> {
    let RuntimeOrigin::Managed {
        provider,
        artifact_sha256,
        artifact_url,
        ownership_id,
        ..
    } = record.origin()
    else {
        return Err(install_error("Expected a managed pending installation"));
    };
    if provider != "python.org"
        || artifact_sha256 != &digest_bytes(&release.sha256)?
        || artifact_url != &release.url
        || path_key(record.root())? != path_key(root)?
    {
        return Err(install_error(
            "Pending installation provenance differs from the current signed release",
        ));
    }
    Ok(Owner {
        schema_version: 1,
        runtime_id: record.id().to_string(),
        ownership_id: ownership_id.to_string(),
        root: root.to_path_buf(),
        sha256: release.sha256.clone(),
    })
}

fn install_error(message: &str) -> Error {
    Error::new(ErrorKind::Install, message)
}

#[cfg(test)]
mod tests {
    use super::{
        App, BOOTSTRAP_PIP_SCRIPT, InstallJournal, Owner, bootstrap_pip, validate_staging_target,
    };
    use crate::arguments::Arguments;
    use clap::Parser;
    use pyrudder_core::runtime::{InstallationId, RuntimeId, RuntimeOrigin, RuntimeRecord};
    use pyrudder_provider_pythonorg::{PythonOrgProvider, extract};
    use std::{
        error::Error,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    fn fixture_app(home: &Path) -> Result<App, Box<dyn Error>> {
        let mut arguments = vec![
            OsString::from("pyrudder"),
            OsString::from("--home"),
            home.into(),
        ];
        for (flag, name) in [
            ("--install-dir", "bin"),
            ("--config-dir", "config"),
            ("--shims-dir", "shims"),
            ("--runtimes-dir", "runtimes"),
            ("--downloads-dir", "downloads"),
            ("--cache-dir", "cache"),
            ("--temp-dir", "temp"),
        ] {
            arguments.push(flag.into());
            arguments.push(home.join(name).into_os_string());
        }
        arguments.push("list".into());
        Ok(App::load(&Arguments::try_parse_from(arguments)?)?)
    }

    fn fixture_record(root: &Path) -> Result<RuntimeRecord, Box<dyn Error>> {
        Ok(RuntimeRecord::new(
            RuntimeId::new("3.13.7".parse()?)?,
            root.to_path_buf(),
            vec![root.join("Scripts")],
            RuntimeOrigin::Managed {
                provider: "python.org".into(),
                artifact_url: "https://www.python.org/ftp/python/3.13.7/python.zip".into(),
                artifact_sha256: [7; 32],
                ownership_id: InstallationId::new(7)?,
            },
        )?)
    }

    #[test]
    fn custom_directory_checks_protected_roots_and_existing_ancestors() -> Result<(), Box<dyn Error>>
    {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("app");
        let app = fixture_app(&home)?;
        assert!(
            app.managed_install_directory(&home.join("runtimes"))
                .is_ok()
        );
        assert!(
            app.managed_install_directory(&temporary.path().join("python"))
                .is_ok()
        );
        assert!(app.managed_install_directory(&home).is_err());
        assert!(
            app.managed_install_directory(Path::new("relative"))
                .is_err()
        );
        assert!(
            app.managed_install_directory(Path::new(r"\\server\share\python"))
                .is_err()
        );
        for name in ["bin", "config", "shims", "downloads", "cache", "temp"] {
            assert!(app.managed_install_directory(&home.join(name)).is_err());
            assert!(
                app.managed_install_directory(&home.join(name).join("python"))
                    .is_err()
            );
        }
        let file = temporary.path().join("ordinary-file");
        fs::write(&file, b"not a directory")?;
        assert!(app.managed_install_directory(&file.join("python")).is_err());
        Ok(())
    }

    #[test]
    fn registered_custom_root_survives_default_changes_but_rejects_wrong_leaf()
    -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let app = fixture_app(&temporary.path().join("app"))?;
        let custom = fs::canonicalize(temporary.path())?
            .join("custom")
            .join("cpython-3.13.7-x64");
        let record = fixture_record(&custom)?;
        assert_eq!(
            super::path_key(&app.registered_managed_root(&record)?)?,
            super::path_key(&custom)?
        );
        let records = std::slice::from_ref(&record);
        let changed = temporary.path().join("changed-default");
        assert_eq!(
            app.installation_root(records, record.id(), Some(&changed))?,
            app.registered_managed_root(&record)?
        );
        let next = RuntimeId::new("3.14.0".parse()?)?;
        assert!(
            app.installation_root(records, &next, custom.parent())
                .is_ok()
        );
        assert!(
            app.installation_root(records, &next, Some(&custom))
                .is_err()
        );
        let wrong_leaf = fixture_record(&temporary.path().join("custom").join("wrong-version"))?;
        assert!(app.registered_managed_root(&wrong_leaf).is_err());
        let protected = fixture_record(&app.location.config_dir.join("cpython-3.13.7-x64"))?;
        assert!(app.registered_managed_root(&protected).is_err());
        Ok(())
    }

    #[test]
    fn recovery_requires_exact_registered_parent_and_attempt_identity() -> Result<(), Box<dyn Error>>
    {
        let temporary = tempfile::tempdir()?;
        let parent = temporary.path().join("custom");
        let attempt = InstallationId::new(9)?;
        let runtime_id = "cpython-3.13.7-x64";
        let journal = InstallJournal {
            owner: Owner {
                schema_version: 1,
                runtime_id: runtime_id.into(),
                ownership_id: InstallationId::new(7)?.to_string(),
                root: parent.join(runtime_id),
                sha256: "07".repeat(32),
            },
            staging: parent.join(format!(".pyrudder-staging-{attempt}")),
        };
        let filename = format!("install-{runtime_id}-{attempt}.json");
        validate_staging_target(&filename, &journal, &parent)?;
        assert!(
            validate_staging_target(&filename, &journal, &temporary.path().join("other")).is_err()
        );
        assert!(
            validate_staging_target(
                &format!("install-{runtime_id}-{:032x}.json", 10),
                &journal,
                &parent
            )
            .is_err()
        );
        let legacy = InstallJournal {
            staging: parent.join(".pyrudder-staging-random"),
            ..journal
        };
        assert!(validate_staging_target(&filename, &legacy, &parent).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "Requires explicit verified local provider cache/downloads; runs only disposable runtimes / 需要显式已验证的本地索引及下载缓存，仅执行临时运行时"]
    fn bootstrap_recreates_missing_launchers_from_bundled_wheel() -> Result<(), Box<dyn Error>> {
        let cache = PathBuf::from(std::env::var("PYRUDDER_TEST_PROVIDER_CACHE")?);
        let downloads = PathBuf::from(std::env::var("PYRUDDER_TEST_PROVIDER_DOWNLOADS")?);
        let minor = std::env::var("PYRUDDER_TEST_PYTHON_MINOR").unwrap_or_else(|_| "3.13".into());
        let releases = PythonOrgProvider::releases(&cache, true)?;
        let release = PythonOrgProvider::select(&releases, &minor)?;
        let archive = PythonOrgProvider::download(&release, &downloads, true)?;
        let temporary = tempfile::Builder::new()
            .prefix("pyrudder-pip-test-")
            .tempdir()?;
        let root = temporary.path().join("runtime");
        fs::create_dir(&root)?;
        extract(&archive, &release.sha256, &root)?;
        let python = root.join("python.exe");
        let pip_version = Command::new(&python)
            .args([
                "-I",
                "-c",
                "import pip,ensurepip; print(pip.__version__, ensurepip.version())",
            ])
            .output()?;
        assert!(
            pip_version.status.success(),
            "The regression requires a preinstalled pip module"
        );
        let launcher_names = [
            "pip.exe".to_owned(),
            "pip3.exe".to_owned(),
            format!("pip{minor}.exe"),
        ];
        for name in &launcher_names {
            let launcher = root.join("Scripts").join(name);
            if launcher.exists() {
                fs::remove_file(launcher)?;
            }
        }
        bootstrap_pip(&root)?;
        for name in &launcher_names {
            let output = Command::new(root.join("Scripts").join(name))
                .arg("--version")
                .output()?;
            assert!(
                output.status.success(),
                "{name} must execute after bootstrap"
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(&format!("python {minor}")));
        }

        // Retry with a missing launcher and hostile caller configuration, confined to this fixture.
        // 删除临时夹具中的启动器后重试，同时模拟可能改变目标或禁止生成启动器的调用方配置。
        fs::remove_file(root.join("Scripts/pip.exe"))?;
        let redirected = temporary.path().join("must-not-be-created");
        let retry = Command::new(&python)
            .args(["-I", "-c", BOOTSTRAP_PIP_SCRIPT])
            .env("PIP_TARGET", &redirected)
            .env("PIP_INDEX_URL", "https://example.invalid/no-network")
            .env("ENSUREPIP_OPTIONS", "altinstall")
            .output()?;
        assert!(
            retry.status.success(),
            "{}",
            String::from_utf8_lossy(&retry.stderr)
        );
        assert!(!redirected.exists());
        for name in &launcher_names {
            assert!(root.join("Scripts").join(name).is_file());
        }
        println!(
            "Python {}: existing pip module, all three launchers, offline retry and configuration isolation passed",
            release.version
        );
        Ok(())
    }
}
