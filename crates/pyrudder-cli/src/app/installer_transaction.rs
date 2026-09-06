//! Bounded, recoverable installer replacement of program and routing files only.
//! 仅针对程序及路由文件的有界、可恢复安装器替换事务。
//!
//! This is not a power-loss-atomic transaction. Python, user configuration, selections and
//! PATH journals are never backup targets, and an unsuccessful restore keeps its evidence.
//! 这不是断电原子事务。Python、用户配置、版本选择和 PATH 日志从不纳入替换范围；
//! 恢复失败时保留备份证据。

use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{CommandExtension, CommandKey, is_reserved},
    state::StateFileSystem,
};
use pyrudder_platform_windows::{
    commands::WindowsCommandCaseMapper,
    publication,
    registry::{Location, PublishedShim, Registry, Snapshot},
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, path_key, sha256_file},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

const BACKUP_DIRECTORY: &str = ".pyrudder-upgrade-backup";
const JOURNAL_FILE: &str = "journal.json";
const MAX_FILE: usize = 128 * 1024 * 1024;
const MAX_JOURNAL: usize = 32 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 131_104;
const REPARSE_POINT: u32 = 0x400;

// Required schema-1 entries from 0.1.0; retain legacy names so unfinished journals stay readable.
// 0.1.0 的 schema-1 必需项；保留旧文件名，确保未完成的旧日志仍可恢复。
// Never enumerate the installation root or rewrite an existing journal's entry order.
// 绝不遍历安装根目录，也不重排已有日志条目。
const FIXED_FILES: &[&str] = &[
    "bin/pyrudder.exe",
    "bin/pyrudder-shim-console.exe",
    "bin/pyrudder-shim-gui.exe",
    "bin/pyrudder-layout.json",
    "README.md",
    "README_EN.md",
    "LICENSE",
    "NOTICE",
    "THIRD-PARTY-NOTICES.txt",
    "BUILD-INFO.json",
    "SBOM.cdx.json",
    "SHA256SUMS.txt",
    "assets/pyrudder-logo.svg",
    "assets/pyrudder-logo.png",
    "assets/banner.txt",
    "unins000.exe",
    "unins000.dat",
    "bin/pyrudder-location.json",
    "config/pyrudder-location.json",
    "shims/pyrudder-location.json",
    "shims/pyrudder-publication.json",
    "config/registry.json",
];

// New transactions always capture these files, but older schema-1 journals may omit them.
// 新事务始终记录这些文件；较早的 schema-1 日志可以不包含它们。
const OPTIONAL_FIXED_FILES: &[&str] = &["README_ZH.md"];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Prepared,
    Restoring,
    RolledBack,
    Committed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    root: PathBuf,
    location: Location,
    from_version: String,
    phase: Phase,
    entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    relative: String,
    sha256: Option<String>,
    size: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationJournal {
    schema_version: u32,
    location: Location,
    entries: Vec<PublishedShim>,
}

fn failure(message: &str) -> Error {
    Error::new(ErrorKind::Install, message).with_hint(
        "Do not delete the upgrade backup. Close PyRudder/Python processes and retry installer recovery. / 请勿删除升级备份；关闭 PyRudder/Python 进程后重试安装器恢复。",
    )
}

fn fixed_location(root: &Path, location: &Location) -> Result<()> {
    location.validate()?;
    if root.file_name().is_none()
        || canonical_key(&location.install_dir)? != canonical_key(&root.join("bin"))?
        || canonical_key(&location.config_dir)? != canonical_key(&root.join("config"))?
        || canonical_key(&location.shims_dir)? != canonical_key(&root.join("shims"))?
    {
        return Err(failure(
            "Installer upgrades require this root's bin, config and shims directories",
        ));
    }
    Ok(())
}

fn canonical_key(path: &Path) -> Result<PathBuf> {
    path_key(&WindowsStateFileSystem.canonical_directory(path)?)
}

fn target(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, name| path.join(name))
}

fn canonical_root(root: &Path) -> Result<DirectoryLease> {
    if root.file_name().is_none() {
        return Err(failure("An installation root must not be a drive root"));
    }
    DirectoryLease::acquire(root, false)
}

fn validate_shim_name(name: &str) -> Result<()> {
    CommandKey::new(name, &WindowsCommandCaseMapper)?;
    let (stem, _) =
        CommandExtension::split(name).ok_or_else(|| failure("Invalid upgrade shim filename"))?;
    if is_reserved(stem) && !name.eq_ignore_ascii_case("pyrudder-dispatch.exe") {
        return Err(failure("Reserved filename in upgrade shim ownership"));
    }
    Ok(())
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_source_version(text: &str) -> bool {
    let parts: Vec<_> = text.split('.').take(4).collect();
    if parts.len() != 3 {
        return false;
    }
    let mut version = [0_u32; 3];
    for (part, output) in parts.into_iter().zip(&mut version) {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return false;
        }
        let Ok(number) = part.parse::<u32>() else {
            return false;
        };
        *output = number;
    }
    version >= [0, 1, 0]
}

fn owned_shims(location: &Location, snapshot: &Snapshot) -> Result<BTreeMap<String, Vec<String>>> {
    let mut records = snapshot.shims.clone();
    records.extend(snapshot.pending_cleanup.clone());
    if let Some(bytes) = WindowsStateFileSystem.read_file(
        &location.shims_dir.join("pyrudder-publication.json"),
        MAX_JOURNAL,
    )? {
        let journal: PublicationJournal = serde_json::from_slice(&bytes)
            .map_err(|_| failure("Invalid shim publication journal during upgrade"))?;
        if journal.schema_version != 1
            || journal.location != *location
            || journal.entries.len() > 131_072
        {
            return Err(failure(
                "Mismatched shim publication journal during upgrade",
            ));
        }
        records.extend(journal.entries);
    }
    let mut owned: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in records {
        validate_shim_name(&record.filename)?;
        record.request()?;
        if !valid_digest(&record.sha256) {
            return Err(failure("Invalid upgrade shim ownership digest"));
        }
        owned
            .entry(
                CommandKey::new(&record.filename, &WindowsCommandCaseMapper)?
                    .as_str()
                    .to_owned(),
            )
            .or_default()
            .push(record.sha256);
    }
    if owned.len() > MAX_ENTRIES - FIXED_FILES.len() - OPTIONAL_FIXED_FILES.len() {
        return Err(failure("Too many owned upgrade shim files"));
    }
    Ok(owned)
}

fn inspect(path: &Path) -> Result<Option<(String, u64)>> {
    let parent = path
        .parent()
        .ok_or_else(|| failure("Missing file parent"))?;
    let _parent = DirectoryLease::acquire(parent, false)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(failure("Cannot inspect upgrade file")),
    };
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || metadata.len() > MAX_FILE as u64
    {
        return Err(failure("Upgrade target is not a bounded ordinary file"));
    }
    Ok(Some((sha256_file(path, MAX_FILE as u64)?, metadata.len())))
}

fn backup_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(format!("{index:06}.bak"))
}

fn save(directory: &Path, journal: &Journal) -> Result<()> {
    let _directory = DirectoryLease::acquire(directory, false)?;
    let bytes = serde_json::to_vec(journal)
        .map_err(|_| failure("Cannot encode installer upgrade journal"))?;
    if bytes.len() > MAX_JOURNAL {
        return Err(failure("Installer upgrade journal exceeds its size limit"));
    }
    WindowsStateFileSystem.write_atomic(&directory.join(JOURNAL_FILE), &bytes)
}

fn load(root: &Path, directory: &Path) -> Result<Journal> {
    let _directory = DirectoryLease::acquire(directory, false)?;
    let bytes = WindowsStateFileSystem
        .read_file(&directory.join(JOURNAL_FILE), MAX_JOURNAL)?
        .ok_or_else(|| failure("Unknown backup directory: upgrade journal is missing"))?;
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|_| failure("Invalid installer upgrade journal"))?;
    if journal.schema_version != 1
        || canonical_key(&journal.root)? != canonical_key(root)?
        || journal.entries.len() < FIXED_FILES.len()
        || journal.entries.len() > MAX_ENTRIES
        || !valid_source_version(&journal.from_version)
    {
        return Err(failure("Mismatched installer upgrade journal"));
    }
    fixed_location(root, &journal.location)?;
    let mut paths = BTreeSet::new();
    let mut total = 0_u64;
    for entry in &journal.entries {
        if !FIXED_FILES.contains(&entry.relative.as_str())
            && !OPTIONAL_FIXED_FILES.contains(&entry.relative.as_str())
        {
            let name = entry
                .relative
                .strip_prefix("shims/")
                .ok_or_else(|| failure("File outside the installer recovery allowlist"))?;
            validate_shim_name(name)?;
        }
        if !paths.insert(path_key(&target(root, &entry.relative))?)
            || entry.size > MAX_FILE as u64
            || entry
                .sha256
                .as_ref()
                .is_some_and(|hash| !valid_digest(hash))
            || (entry.sha256.is_none() && entry.size != 0)
        {
            return Err(failure("Invalid or duplicate installer recovery entry"));
        }
        total = total
            .checked_add(entry.size)
            .ok_or_else(|| failure("Installer backup size overflow"))?;
    }
    if total > MAX_TOTAL
        || FIXED_FILES.iter().any(|required| {
            !journal
                .entries
                .iter()
                .any(|entry| entry.relative == *required)
        })
    {
        return Err(failure(
            "Incomplete or oversized installer recovery allowlist",
        ));
    }
    Ok(journal)
}

fn inventory(directory: &Path, journal: &Journal) -> Result<()> {
    let _directory = DirectoryLease::acquire(directory, false)?;
    let allowed: BTreeSet<_> = std::iter::once(JOURNAL_FILE.to_owned())
        .chain(
            journal
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.sha256.is_some())
                .map(|(index, _)| format!("{index:06}.bak")),
        )
        .collect();
    let items = fs::read_dir(directory).map_err(|_| failure("Cannot inspect backup contents"))?;
    for (count, item) in items.enumerate() {
        let item = item.map_err(|_| failure("Cannot inspect a backup item"))?;
        let name = item.file_name();
        if count > MAX_ENTRIES || !name.to_str().is_some_and(|name| allowed.contains(name)) {
            return Err(failure(
                "Unknown files in installer backup; nothing was removed",
            ));
        }
        let metadata = fs::symlink_metadata(item.path())
            .map_err(|_| failure("Cannot inspect backup item type"))?;
        if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err(failure(
                "Unsafe file in installer backup; nothing was removed",
            ));
        }
    }
    Ok(())
}

fn cleanup(directory: &Path, journal: &Journal) -> Result<()> {
    let directory_anchor = DirectoryLease::acquire(directory, false)?;
    inventory(directory, journal)?;
    for (index, entry) in journal.entries.iter().enumerate() {
        if entry.sha256.is_some() {
            remove_ordinary(&backup_path(directory, index))?;
        }
    }
    // Keep the terminal phase until all data files have been removed.
    // 删除全部备份数据文件之前，一直保留终态日志。
    remove_ordinary(&directory.join(JOURNAL_FILE))?;
    drop(directory_anchor);
    if fs::remove_dir(directory).is_err() {
        // A held directory must not lose its terminal marker and block the working version.
        // 目录被占用时不可丢失终态标记，否则会阻止已经可用的版本运行。
        save(directory, journal)?;
        return Err(failure("Cannot remove the empty upgrade backup"));
    }
    Ok(())
}

fn remove_ordinary(path: &Path) -> Result<()> {
    if inspect(path)?.is_some() {
        fs::remove_file(path)
            .map_err(|_| failure("Upgrade file is occupied; recovery is retained"))?;
    }
    Ok(())
}

fn prepare_entries(root: &Path, location: &Location, snapshot: &Snapshot) -> Result<Vec<Entry>> {
    let owned = owned_shims(location, snapshot)?;
    let mut names: BTreeSet<_> = owned.keys().cloned().collect();
    for entry in publication::desired_index(location, snapshot)? {
        validate_shim_name(&entry.filename)?;
        names.insert(
            CommandKey::new(&entry.filename, &WindowsCommandCaseMapper)?
                .as_str()
                .to_owned(),
        );
    }
    let mut relatives: Vec<_> = FIXED_FILES
        .iter()
        .chain(OPTIONAL_FIXED_FILES)
        .map(|value| (*value).to_owned())
        .collect();
    relatives.extend(names.into_iter().map(|name| format!("shims/{name}")));
    if relatives.len() > MAX_ENTRIES {
        return Err(failure("Too many installer backup entries"));
    }
    let mut entries = Vec::new();
    let mut total = 0_u64;
    for relative in relatives {
        let existing = inspect(&target(root, &relative))?;
        if let Some(name) = relative.strip_prefix("shims/") {
            if !FIXED_FILES.contains(&relative.as_str()) {
                if let Some((digest, _)) = &existing {
                    if !owned
                        .get(name)
                        .is_some_and(|hashes| hashes.contains(digest))
                    {
                        return Err(failure(
                            "Modified or foreign shim cannot be backed up as owned",
                        ));
                    }
                }
            }
        }
        let (sha256, size) = existing.map_or((None, 0), |(hash, size)| (Some(hash), size));
        total = total
            .checked_add(size)
            .ok_or_else(|| failure("Installer backup size overflow"))?;
        if total > MAX_TOTAL {
            return Err(failure(
                "Installer backup exceeds the bounded recovery size limit",
            ));
        }
        entries.push(Entry {
            relative,
            sha256,
            size,
        });
    }
    Ok(entries)
}

/// Captures only the explicit installer replacement set before Inno writes any file.
/// 在 Inno 写入任何文件之前，仅保存明确的安装器替换集合。
pub(super) fn prepare(root: &Path, location: &Location, from_version: &str) -> Result<()> {
    let root_anchor = canonical_root(root)?;
    let root = root_anchor.path();
    fixed_location(root, location)?;
    if !valid_source_version(from_version) {
        return Err(failure("Invalid source version for installer backup"));
    }
    let _access = FileLease::acquire(&location.config_dir.join("installer.lock"), true, true)?;
    let transaction = Registry::new(&location.config_dir)?.transaction()?;
    let directory = root.join(BACKUP_DIRECTORY);
    if directory
        .try_exists()
        .map_err(|_| failure("Cannot inspect upgrade backup"))?
    {
        let old = load(root, &directory)?;
        if old.phase != Phase::Committed {
            return Err(failure(
                "An unfinished installer upgrade must be recovered first",
            ));
        }
        cleanup(&directory, &old)?;
    }
    let entries = prepare_entries(root, location, &transaction.snapshot)?;
    fs::create_dir(&directory).map_err(|_| failure("Cannot create a new installer backup"))?;
    let mut journal = Journal {
        schema_version: 1,
        root: root.to_path_buf(),
        location: location.clone(),
        from_version: from_version.to_owned(),
        phase: Phase::Preparing,
        entries,
    };
    if let Err(error) = save(&directory, &journal) {
        // Only this newly created empty directory may be removed; never clean an unknown root.
        // 只能删除本次新建的空备份目录；绝不清理未知根目录。
        let _ = fs::remove_dir(&directory);
        return Err(error);
    }
    let copied = copy_backups(root, &directory, &journal);
    if let Err(error) = copied {
        // Preparation never changed the original installation, so discard only our own backup.
        // 准备阶段尚未更改原安装，因此仅清理本事务创建的备份。
        let _ = cleanup(&directory, &journal);
        return Err(error);
    }
    journal.phase = Phase::Prepared;
    if let Err(error) = save(&directory, &journal) {
        let _ = cleanup(&directory, &journal);
        return Err(error);
    }
    Ok(())
}

fn copy_backups(root: &Path, directory: &Path, journal: &Journal) -> Result<()> {
    for (index, entry) in journal.entries.iter().enumerate() {
        if let Some(digest) = &entry.sha256 {
            let bytes = WindowsStateFileSystem
                .read_file(&target(root, &entry.relative), MAX_FILE)?
                .ok_or_else(|| failure("Upgrade source disappeared while preparing backup"))?;
            let backup = backup_path(directory, index);
            WindowsStateFileSystem.write_atomic(&backup, &bytes)?;
            if bytes.len() as u64 != entry.size || sha256_file(&backup, MAX_FILE as u64)? != *digest
            {
                return Err(failure("Upgrade source changed while preparing backup"));
            }
        }
    }
    Ok(())
}

fn validate_backups(directory: &Path, journal: &Journal) -> Result<()> {
    inventory(directory, journal)?;
    for (index, entry) in journal.entries.iter().enumerate() {
        if let Some(expected) = &entry.sha256 {
            if inspect(&backup_path(directory, index))? != Some((expected.clone(), entry.size)) {
                return Err(failure(
                    "Installer backup is missing or changed; restore was not started",
                ));
            }
        }
    }
    Ok(())
}

fn restore_entry(root: &Path, directory: &Path, index: usize, entry: &Entry) -> Result<()> {
    let destination = target(root, &entry.relative);
    let parent = destination
        .parent()
        .ok_or_else(|| failure("Invalid recovery parent"))?;
    let _parent = DirectoryLease::acquire(parent, false)?;
    if entry.sha256.is_some() {
        let bytes = WindowsStateFileSystem
            .read_file(&backup_path(directory, index), MAX_FILE)?
            .ok_or_else(|| failure("Installer backup disappeared during restore"))?;
        WindowsStateFileSystem.write_atomic(&destination, &bytes)?;
    } else {
        remove_ordinary(&destination)?;
    }
    Ok(())
}

/// Restores files and retains the old version until Inno acknowledges registry recovery.
/// 恢复文件并保留旧版本依据，直到 Inno 确认已恢复卸载注册信息。
pub(super) fn recover(root: &Path) -> Result<Option<String>> {
    let root_anchor = canonical_root(root)?;
    let root = root_anchor.path();
    let directory = root.join(BACKUP_DIRECTORY);
    if !directory
        .try_exists()
        .map_err(|_| failure("Cannot inspect upgrade backup"))?
    {
        return Ok(None);
    }
    let _access = FileLease::acquire(&target(root, "config/installer.lock"), true, true)?;
    let _registry = FileLease::acquire(&target(root, "config/registry.lock"), true, true)?;
    let mut journal = load(root, &directory)?;
    if matches!(journal.phase, Phase::Committed | Phase::Preparing) {
        // No replacement began while preparing; committed data must never be rolled back.
        // preparing 阶段尚未替换；已提交的新版本绝不回滚成旧版本。
        cleanup(&directory, &journal)?;
        return Ok(None);
    }
    if journal.phase == Phase::RolledBack {
        return Ok(Some(journal.from_version));
    }
    validate_backups(&directory, &journal)?;
    journal.phase = Phase::Restoring;
    save(&directory, &journal)?;
    // Restore the old routing snapshot last, after all of its physical entries exist again.
    // 最后恢复旧路由快照，确保它对应的物理入口已全部恢复。
    for (index, entry) in journal.entries.iter().enumerate() {
        if entry.relative != "config/registry.json" {
            restore_entry(root, &directory, index, entry)?;
        }
    }
    for (index, entry) in journal.entries.iter().enumerate() {
        if entry.relative == "config/registry.json" {
            restore_entry(root, &directory, index, entry)?;
        }
    }
    journal.phase = Phase::RolledBack;
    save(&directory, &journal)?;
    Ok(Some(journal.from_version))
}

/// Discards recovery evidence only after the installer restored its exact uninstall key.
/// 仅在安装器恢复自身的精确卸载注册项之后，才清除恢复证据。
pub(super) fn acknowledge_recovery(root: &Path) -> Result<()> {
    let root_anchor = canonical_root(root)?;
    let root = root_anchor.path();
    let directory = root.join(BACKUP_DIRECTORY);
    if !directory
        .try_exists()
        .map_err(|_| failure("Cannot inspect upgrade backup"))?
    {
        return Ok(());
    }
    let _access = FileLease::acquire(&target(root, "config/installer.lock"), true, true)?;
    let _registry = FileLease::acquire(&target(root, "config/registry.lock"), true, true)?;
    let journal = load(root, &directory)?;
    if journal.phase != Phase::RolledBack {
        return Err(failure(
            "Installer registry recovery has not been acknowledged",
        ));
    }
    cleanup(&directory, &journal)
}

// Isolated file-transaction tests acknowledge recovery without touching any registry key.
// 隔离文件事务测试直接确认恢复，不访问任何注册表项。
#[cfg(test)]
fn rollback(root: &Path) -> Result<()> {
    if recover(root)?.is_some() {
        acknowledge_recovery(root)?;
    }
    Ok(())
}

/// Commits only after setup and Inno's installation have both succeeded.
/// 仅在 setup 与 Inno 安装都成功之后确认提交。
pub(super) fn commit(root: &Path) -> Result<()> {
    let root_anchor = canonical_root(root)?;
    let root = root_anchor.path();
    let directory = root.join(BACKUP_DIRECTORY);
    let _access = FileLease::acquire(&target(root, "config/installer.lock"), true, true)?;
    let _registry = FileLease::acquire(&target(root, "config/registry.lock"), true, true)?;
    let mut journal = load(root, &directory)?;
    if !matches!(journal.phase, Phase::Prepared | Phase::Committed) {
        return Err(failure("Installer upgrade is not ready to commit"));
    }
    if journal.phase != Phase::Committed {
        validate_backups(&directory, &journal)?;
        journal.phase = Phase::Committed;
        save(&directory, &journal)?;
    }
    if cleanup(&directory, &journal).is_err() {
        eprintln!(
            "Upgrade committed; backup cleanup is deferred. Do not restore the old version. / 升级已提交，备份清理已延期；请勿恢复旧版本。"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};

    fn fixture() -> Result<(tempfile::TempDir, Location)> {
        let root =
            tempfile::tempdir().map_err(|_| failure("Cannot create isolated test directory"))?;
        for name in ["bin", "config", "shims", "assets", "runtimes"] {
            fs::create_dir(root.path().join(name)).map_err(|_| failure("Cannot create fixture"))?;
        }
        let location = Location {
            schema_version: 1,
            install_dir: root.path().join("bin"),
            config_dir: root.path().join("config"),
            shims_dir: root.path().join("shims"),
        };
        for name in FIXED_FILES {
            if !name.starts_with("config/") && !name.starts_with("shims/") {
                fs::write(root.path().join(name), format!("old:{name}"))
                    .map_err(|_| failure("Cannot populate fixture"))?;
            }
        }
        fs::remove_file(location.install_dir.join("pyrudder-location.json"))
            .map_err(|_| failure("Cannot reset fixture locator"))?;
        for directory in [
            &location.install_dir,
            &location.config_dir,
            &location.shims_dir,
        ] {
            location.publish(directory)?;
        }
        Registry::new(&location.config_dir)?
            .transaction()?
            .commit()?;
        Ok((root, location))
    }

    #[test]
    fn rollback_restores_programs_and_never_touches_user_data() -> Result<()> {
        let (root, location) = fixture()?;
        let preserved = [
            "config/config.toml",
            "config/global-version",
            "runtimes/notes.txt",
        ];
        for name in preserved {
            fs::write(root.path().join(name), "user-data").map_err(|_| failure("Fixture write"))?;
        }
        prepare(root.path(), &location, "0.1.0")?;
        fs::write(location.install_dir.join("pyrudder.exe"), "new-program")
            .map_err(|_| failure("Fixture write"))?;
        fs::write(root.path().join("unins000.dat"), "new-uninstaller")
            .map_err(|_| failure("Fixture write"))?;
        rollback(root.path())?;
        rollback(root.path())?;
        assert_eq!(
            fs::read(root.path().join("bin/pyrudder.exe")).ok(),
            Some(b"old:bin/pyrudder.exe".to_vec())
        );
        assert_eq!(
            fs::read(root.path().join("unins000.dat")).ok(),
            Some(b"old:unins000.dat".to_vec())
        );
        for name in preserved {
            assert_eq!(
                fs::read(root.path().join(name)).ok(),
                Some(b"user-data".to_vec())
            );
        }
        assert!(!root.path().join(BACKUP_DIRECTORY).exists());
        Ok(())
    }

    #[test]
    fn legacy_journal_without_chinese_readme_preserves_backup_indices() -> Result<()> {
        let (root, location) = fixture()?;
        let shim = location.shims_dir.join("python.exe");
        fs::write(&shim, "old-shim").map_err(|_| failure("Fixture write"))?;
        let registry = Registry::new(&location.config_dir)?;
        let mut transaction = registry.transaction()?;
        transaction.snapshot.shims.push(PublishedShim {
            filename: "python.exe".into(),
            request_name: "python".into(),
            exact: false,
            gui: false,
            sha256: sha256_file(&shim, MAX_FILE as u64)?,
        });
        transaction.commit()?;
        let original_registry = fs::read(location.config_dir.join("registry.json")).ok();
        let mut entries = prepare_entries(root.path(), &location, &registry.load()?)?;
        // Construct the old layout before writing indexed blobs, as the 0.1.0 helper did.
        // 在写入编号备份前构造旧布局，与 0.1.0 辅助程序保持一致。
        entries.retain(|entry| !OPTIONAL_FIXED_FILES.contains(&entry.relative.as_str()));
        let original_order: Vec<_> = entries.iter().map(|entry| entry.relative.clone()).collect();
        let directory = root.path().join(BACKUP_DIRECTORY);
        fs::create_dir(&directory).map_err(|_| failure("Fixture directory"))?;
        let journal = Journal {
            schema_version: 1,
            root: root.path().to_path_buf(),
            location: location.clone(),
            from_version: "0.1.0".into(),
            phase: Phase::Prepared,
            entries,
        };
        save(&directory, &journal)?;
        copy_backups(root.path(), &directory, &journal)?;
        assert_eq!(
            load(root.path(), &directory)?
                .entries
                .into_iter()
                .map(|entry| entry.relative)
                .collect::<Vec<_>>(),
            original_order
        );
        for name in ["README.md", "README_EN.md", "config/registry.json"] {
            fs::write(target(root.path(), name), "new-content")
                .map_err(|_| failure("Fixture write"))?;
        }
        fs::write(&shim, "new-shim").map_err(|_| failure("Fixture write"))?;
        fs::write(root.path().join("README_ZH.md"), "not-in-old-transaction")
            .map_err(|_| failure("Fixture write"))?;
        rollback(root.path())?;
        for name in ["README.md", "README_EN.md"] {
            assert_eq!(
                fs::read(root.path().join(name)).ok(),
                Some(format!("old:{name}").into_bytes())
            );
        }
        assert_eq!(fs::read(&shim).ok(), Some(b"old-shim".to_vec()));
        assert_eq!(
            fs::read(location.config_dir.join("registry.json")).ok(),
            original_registry
        );
        assert_eq!(
            fs::read(root.path().join("README_ZH.md")).ok(),
            Some(b"not-in-old-transaction".to_vec())
        );
        assert!(!directory.exists());
        Ok(())
    }

    #[test]
    fn rollback_tracks_optional_readme_presence_and_preserves_legacy_readmes() -> Result<()> {
        for already_present in [false, true] {
            let (root, location) = fixture()?;
            let chinese = root.path().join("README_ZH.md");
            if already_present {
                fs::write(&chinese, "old-chinese").map_err(|_| failure("Fixture write"))?;
            }
            prepare(root.path(), &location, "0.1.0")?;
            let journal = load(root.path(), &root.path().join(BACKUP_DIRECTORY))?;
            let entry = journal
                .entries
                .iter()
                .find(|entry| entry.relative == "README_ZH.md")
                .ok_or_else(|| failure("New transaction omitted the Chinese README"))?;
            assert_eq!(entry.sha256.is_some(), already_present);
            if !already_present {
                assert_eq!(entry.size, 0);
            }
            fs::write(&chinese, "new-chinese").map_err(|_| failure("Fixture write"))?;
            fs::write(root.path().join("README.md"), "new-english")
                .map_err(|_| failure("Fixture write"))?;
            fs::remove_file(root.path().join("README_EN.md"))
                .map_err(|_| failure("Fixture removal"))?;
            rollback(root.path())?;
            assert_eq!(
                fs::read(&chinese).ok(),
                already_present.then(|| b"old-chinese".to_vec())
            );
            for name in ["README.md", "README_EN.md"] {
                assert_eq!(
                    fs::read(root.path().join(name)).ok(),
                    Some(format!("old:{name}").into_bytes())
                );
            }
        }
        Ok(())
    }

    #[test]
    fn optional_readme_does_not_allow_unknown_files_or_missing_legacy_entries() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        let directory = root.path().join(BACKUP_DIRECTORY);
        let mut journal = load(root.path(), &directory)?;
        fs::write(root.path().join("README.md"), "new-english")
            .map_err(|_| failure("Fixture write"))?;
        let unknown = root.path().join("README_FR.md");
        fs::write(&unknown, "unrelated").map_err(|_| failure("Fixture write"))?;
        journal.entries.push(Entry {
            relative: "README_FR.md".into(),
            sha256: None,
            size: 0,
        });
        save(&directory, &journal)?;
        assert!(rollback(root.path()).is_err());
        assert_eq!(fs::read(&unknown).ok(), Some(b"unrelated".to_vec()));
        assert_eq!(
            fs::read(root.path().join("README.md")).ok(),
            Some(b"new-english".to_vec())
        );
        journal.entries.pop();
        journal
            .entries
            .retain(|entry| entry.relative != "README_EN.md");
        save(&directory, &journal)?;
        assert!(rollback(root.path()).is_err());
        assert!(directory.exists());
        Ok(())
    }

    #[test]
    fn owned_shims_and_snapshot_are_restored_together() -> Result<()> {
        let (root, location) = fixture()?;
        let shim = location.shims_dir.join("python.exe");
        fs::write(&shim, "old-shim").map_err(|_| failure("Fixture write"))?;
        let mut transaction = Registry::new(&location.config_dir)?.transaction()?;
        transaction.snapshot.shims.push(PublishedShim {
            filename: "python.exe".into(),
            request_name: "python".into(),
            exact: false,
            gui: false,
            sha256: sha256_file(&shim, MAX_FILE as u64)?,
        });
        transaction.commit()?;
        let original = fs::read(location.config_dir.join("registry.json")).ok();
        prepare(root.path(), &location, "0.1.0")?;
        fs::write(&shim, "new-shim").map_err(|_| failure("Fixture write"))?;
        fs::write(
            location.config_dir.join("registry.json"),
            "broken-new-state",
        )
        .map_err(|_| failure("Fixture write"))?;
        rollback(root.path())?;
        assert_eq!(fs::read(shim).ok(), Some(b"old-shim".to_vec()));
        assert_eq!(
            fs::read(location.config_dir.join("registry.json")).ok(),
            original
        );
        Ok(())
    }

    #[test]
    fn occupied_restore_keeps_complete_backup_for_retry() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        let program = location.install_dir.join("pyrudder.exe");
        fs::write(&program, "new-program").map_err(|_| failure("Fixture write"))?;
        let occupied = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&program)
            .map_err(|_| failure("Cannot acquire fixture lock"))?;
        assert!(rollback(root.path()).is_err());
        assert!(
            root.path()
                .join(BACKUP_DIRECTORY)
                .join(JOURNAL_FILE)
                .exists()
        );
        drop(occupied);
        rollback(root.path())?;
        assert_eq!(
            fs::read(program).ok(),
            Some(b"old:bin/pyrudder.exe".to_vec())
        );
        Ok(())
    }

    #[test]
    fn corrupted_backup_is_rejected_before_any_restore() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        fs::write(location.install_dir.join("pyrudder.exe"), "new-program")
            .map_err(|_| failure("Fixture write"))?;
        fs::write(
            backup_path(&root.path().join(BACKUP_DIRECTORY), 1),
            "corrupt",
        )
        .map_err(|_| failure("Fixture write"))?;
        assert!(rollback(root.path()).is_err());
        assert_eq!(
            fs::read(location.install_dir.join("pyrudder.exe")).ok(),
            Some(b"new-program".to_vec())
        );
        Ok(())
    }

    #[test]
    fn commit_keeps_new_files_and_deferred_cleanup_never_restores_old_files() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        fs::write(location.install_dir.join("pyrudder.exe"), "new-program")
            .map_err(|_| failure("Fixture write"))?;
        let unknown = root.path().join(BACKUP_DIRECTORY).join("foreign.txt");
        fs::write(&unknown, "unrelated").map_err(|_| failure("Fixture write"))?;
        // Unknown data must block commit, not be silently deleted.
        // 不明数据必须阻止提交，不可静默删除。
        assert!(commit(root.path()).is_err());
        fs::remove_file(&unknown).map_err(|_| failure("Fixture removal"))?;
        let held_path = backup_path(&root.path().join(BACKUP_DIRECTORY), 0);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(held_path)
            .map_err(|_| failure("Cannot lock backup fixture"))?;
        commit(root.path())?;
        assert_eq!(
            load(root.path(), &root.path().join(BACKUP_DIRECTORY))?.phase,
            Phase::Committed
        );
        drop(held);
        rollback(root.path())?;
        assert_eq!(
            fs::read(location.install_dir.join("pyrudder.exe")).ok(),
            Some(b"new-program".to_vec())
        );
        Ok(())
    }

    #[test]
    fn mismatched_root_and_incomplete_preparation_fail_closed() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        assert!(prepare(root.path(), &location, "0.1.0").is_err());
        let directory = root.path().join(BACKUP_DIRECTORY);
        let mut journal = load(root.path(), &directory)?;
        journal.root = root.path().join("unrelated");
        save(&directory, &journal)?;
        assert!(rollback(root.path()).is_err());
        journal.root = root.path().to_path_buf();
        journal.phase = Phase::Preparing;
        save(&directory, &journal)?;
        fs::remove_file(backup_path(&directory, 0)).map_err(|_| failure("Fixture removal"))?;
        rollback(root.path())?;
        assert!(!directory.exists());
        Ok(())
    }

    #[test]
    fn modified_owned_shim_blocks_preparation_without_creating_backup() -> Result<()> {
        let (root, location) = fixture()?;
        let shim = location.shims_dir.join("python.exe");
        fs::write(&shim, "owned").map_err(|_| failure("Fixture write"))?;
        let mut transaction = Registry::new(&location.config_dir)?.transaction()?;
        transaction.snapshot.shims.push(PublishedShim {
            filename: "python.exe".into(),
            request_name: "python".into(),
            exact: false,
            gui: false,
            sha256: sha256_file(&shim, MAX_FILE as u64)?,
        });
        transaction.commit()?;
        fs::write(&shim, "foreign-change").map_err(|_| failure("Fixture write"))?;
        assert!(prepare(root.path(), &location, "0.1.0").is_err());
        assert!(!root.path().join(BACKUP_DIRECTORY).exists());
        assert_eq!(fs::read(&shim).ok(), Some(b"foreign-change".to_vec()));
        Ok(())
    }

    #[test]
    fn journal_traversal_is_rejected_before_any_restore() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        let directory = root.path().join(BACKUP_DIRECTORY);
        let mut journal = load(root.path(), &directory)?;
        journal.entries.push(Entry {
            relative: "shims/../../unrelated.exe".into(),
            sha256: None,
            size: 0,
        });
        save(&directory, &journal)?;
        assert!(rollback(root.path()).is_err());
        assert!(directory.exists());
        Ok(())
    }

    #[test]
    fn occupied_backup_directory_keeps_terminal_marker_after_commit() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        let directory = root.path().join(BACKUP_DIRECTORY);
        let held = DirectoryLease::acquire(&directory, false)?;
        commit(root.path())?;
        assert_eq!(load(root.path(), &directory)?.phase, Phase::Committed);
        drop(held);
        rollback(root.path())?;
        assert!(!directory.exists());
        Ok(())
    }

    #[test]
    fn recovery_repeats_old_version_until_installer_acknowledges_it() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        assert!(acknowledge_recovery(root.path()).is_err());
        fs::write(location.install_dir.join("pyrudder.exe"), "new-program")
            .map_err(|_| failure("Fixture write"))?;
        assert_eq!(recover(root.path())?, Some("0.1.0".into()));
        let directory = root.path().join(BACKUP_DIRECTORY);
        assert_eq!(load(root.path(), &directory)?.phase, Phase::RolledBack);
        assert_eq!(recover(root.path())?, Some("0.1.0".into()));
        assert!(prepare(root.path(), &location, "0.1.0").is_err());
        assert!(commit(root.path()).is_err());
        assert_eq!(
            fs::read(location.install_dir.join("pyrudder.exe")).ok(),
            Some(b"old:bin/pyrudder.exe".to_vec())
        );
        acknowledge_recovery(root.path())?;
        acknowledge_recovery(root.path())?;
        assert_eq!(recover(root.path())?, None);
        assert!(!directory.exists());
        Ok(())
    }

    #[test]
    fn preparing_and_committed_recovery_do_not_request_registry_changes() -> Result<()> {
        let (root, location) = fixture()?;
        prepare(root.path(), &location, "0.1.0")?;
        let directory = root.path().join(BACKUP_DIRECTORY);
        let mut journal = load(root.path(), &directory)?;
        journal.phase = Phase::Preparing;
        save(&directory, &journal)?;
        assert_eq!(recover(root.path())?, None);
        prepare(root.path(), &location, "0.1.0")?;
        let held = DirectoryLease::acquire(&directory, false)?;
        commit(root.path())?;
        drop(held);
        assert!(acknowledge_recovery(root.path()).is_err());
        assert_eq!(recover(root.path())?, None);
        Ok(())
    }

    #[test]
    fn recovery_version_rejects_nonstable_versions_and_output_injection() -> Result<()> {
        for valid in ["0.1.0", "0.1.10", "1.0.0", "4294967295.0.0"] {
            assert!(valid_source_version(valid));
        }
        let (root, location) = fixture()?;
        for invalid in [
            "",
            "0.0.9",
            "0.1.0-alpha.8",
            "0.01.0",
            "1.2",
            "1.2.3.4",
            "4294967296.0.0",
            "0.1.0\nPYRUDDER_RESTORED_VERSION=9.9.9",
            "0.1.0\r",
            "0.1.0+build",
        ] {
            assert!(!valid_source_version(invalid));
            assert!(prepare(root.path(), &location, invalid).is_err());
        }
        prepare(root.path(), &location, "0.1.0")?;
        let directory = root.path().join(BACKUP_DIRECTORY);
        let mut journal = load(root.path(), &directory)?;
        journal.from_version = "0.1.0\nINJECTED".into();
        save(&directory, &journal)?;
        assert!(recover(root.path()).is_err());
        Ok(())
    }
}
