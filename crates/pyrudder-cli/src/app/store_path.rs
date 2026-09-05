//! Narrow, reversible removal of one user's duplicated machine `WindowsApps` entry.
//! 精确、可撤销地移除某个用户重复存在于系统 PATH 的 `WindowsApps` 项。

mod state;

use super::{App, usage};
use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    elevation,
    environment::notify_environment_change,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease},
};
use serde_json::{Value, json};
use state::{JOURNAL_LIMIT, Journal, PathValue, Phase, ValueKind, entries, same_entry};
use std::{ffi::OsString, io::ErrorKind as IoErrorKind, path::Path};
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ},
};

const MACHINE_ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const PROFILES: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
const JOURNAL_NAME: &str = "store-path.json";

impl App {
    pub(super) fn has_store_path_ownership(&self) -> Result<bool> {
        if WindowsStateFileSystem
            .read_file(&self.location.config_dir.join(JOURNAL_NAME), JOURNAL_LIMIT)?
            .is_none()
        {
            return Ok(false);
        }
        let identity = Identity::read(&elevation::current_user_sid()?)?;
        Ok(self
            .read_store_journal(&identity)?
            .is_some_and(|journal| journal.phase != Phase::Restored))
    }

    pub(super) fn repair_store_path(
        &self,
        restore: bool,
        owner_sid: Option<&str>,
    ) -> Result<Value> {
        if restore
            && WindowsStateFileSystem
                .read_file(&self.location.config_dir.join(JOURNAL_NAME), JOURNAL_LIMIT)?
                .is_none()
        {
            return Ok(report(false, true));
        }
        if let Some(sid) = owner_sid {
            let identity = Identity::read(sid)?;
            let changed = if restore {
                self.restore_store_entry(&identity)?
            } else {
                self.remove_store_duplicate(&identity)?
            };
            return Ok(report(changed, restore));
        }
        let sid = elevation::current_user_sid()?;
        let identity = Identity::read(&sid)?;
        let original = read_machine_path()?;
        let current = decoded(original.as_ref())?;
        if restore {
            let Some(journal) = self.read_store_journal(&identity)? else {
                return Ok(report(false, true));
            };
            if journal.phase == Phase::Restored || self.finish_restored_entry(&identity)? {
                return Ok(report(false, true));
            }
        } else if !current
            .as_ref()
            .is_some_and(|value| value.contains(&identity.target))
        {
            // An unaffected installation remains completely read-only and does not prompt UAC.
            // 未受影响的安装完全只读，不触发 UAC。
            return Ok(report(false, false));
        }
        let program =
            std::env::current_exe().map_err(|_| usage("Cannot locate the installed CLI"))?;
        let code = elevation::run_elevated(&program, &self.store_repair_arguments(&sid, restore)?)?;
        if code != 0 {
            return Err(Error::new(
                ErrorKind::Permission,
                format!("Windows Store PATH repair failed with exit code {code}"),
            )
            .with_hint("No terminal initialization command is needed. Rerun the installer and approve its administrator request."));
        }
        let published = read_machine_path()?;
        let contains =
            decoded(published.as_ref())?.is_some_and(|value| value.contains(&identity.target));
        if contains != restore {
            return Err(usage(
                "Windows Store PATH changed again during repair; retry the command",
            ));
        }
        Ok(report(original != published, restore))
    }

    fn store_repair_arguments(&self, sid: &str, restore: bool) -> Result<Vec<OsString>> {
        let paths = &self.configuration.paths;
        let home = paths
            .install_dir
            .parent()
            .ok_or_else(|| usage("CLI installation has no parent directory"))?;
        let mut arguments = Vec::new();
        // Pin all effective locations; another administrator's environment cannot redirect repair.
        // 固定所有生效路径，其他管理员账户的环境变量不能重定向修复。
        for (flag, path) in [
            ("--home", home),
            ("--config-dir", paths.config_dir.as_path()),
            ("--install-dir", paths.install_dir.as_path()),
            ("--shims-dir", paths.shims_dir.as_path()),
            ("--runtimes-dir", paths.runtimes_dir.as_path()),
            ("--downloads-dir", paths.downloads_dir.as_path()),
            ("--cache-dir", paths.cache_dir.as_path()),
            ("--temp-dir", paths.temp_dir.as_path()),
        ] {
            arguments.push(OsString::from(flag));
            arguments.push(path.as_os_str().to_owned());
        }
        arguments.extend([
            OsString::from("__store-path"),
            OsString::from("--owner-sid"),
            OsString::from(sid),
        ]);
        if restore {
            arguments.push(OsString::from("--restore"));
        }
        Ok(arguments)
    }

    fn read_store_journal(&self, identity: &Identity) -> Result<Option<Journal>> {
        let Some(bytes) = WindowsStateFileSystem
            .read_file(&self.location.config_dir.join(JOURNAL_NAME), JOURNAL_LIMIT)?
        else {
            return Ok(None);
        };
        let journal: Journal = serde_json::from_slice(&bytes)
            .map_err(|_| usage("Invalid Windows Store PATH repair journal"))?;
        journal.validate(&identity.sid, &identity.target)?;
        if journal.install_dir != plain_directory(&self.location.install_dir)?
            || journal.shims_dir != plain_directory(&self.location.shims_dir)?
        {
            return Err(usage(
                "Windows Store PATH repair journal belongs to another installation",
            ));
        }
        Ok(Some(journal))
    }

    fn write_store_journal(&self, journal: &Journal) -> Result<()> {
        let bytes = serde_json::to_vec(journal)
            .map_err(|_| usage("Cannot encode Windows Store PATH repair journal"))?;
        if bytes.len() > JOURNAL_LIMIT {
            return Err(usage(
                "Windows Store PATH repair journal exceeds the size limit",
            ));
        }
        WindowsStateFileSystem.write_atomic(&self.location.config_dir.join(JOURNAL_NAME), &bytes)
    }

    fn verify_machine_installation_path(&self) -> Result<()> {
        let Some(value) = decoded(read_machine_path()?.as_ref())? else {
            return Err(usage(
                "The machine PATH is empty after PyRudder publication",
            ));
        };
        let machine_entries = entries(&value.text);
        for path in [&self.location.install_dir, &self.location.shims_dir] {
            let path = plain_directory(path)?;
            if !machine_entries.iter().any(|entry| same_entry(entry, &path)) {
                return Err(usage(
                    "This installation's bin and shims must be present in machine PATH",
                ));
            }
        }
        Ok(())
    }

    fn remove_store_duplicate(&self, identity: &Identity) -> Result<bool> {
        let initial = decoded(read_machine_path()?.as_ref())?;
        if !initial.is_some_and(|value| value.contains(&identity.target)) {
            return Ok(false);
        }
        self.verify_machine_installation_path()?;
        let _directory = DirectoryLease::acquire(&self.location.config_dir, false)?;
        let _lock = FileLease::acquire(
            &self.location.config_dir.join("store-path.lock"),
            true,
            true,
        )?;
        self.read_store_journal(identity)?;
        let key = machine_key(true)?;
        let original = read_raw_path(&key)?;
        let Some(before) = decoded(original.as_ref())? else {
            return Ok(false);
        };
        if !before.contains(&identity.target) {
            return Ok(false);
        }
        let after = before.without(&identity.target);
        let mut journal = Journal {
            schema_version: 1,
            owner_sid: identity.sid.clone(),
            target: identity.target.clone(),
            install_dir: plain_directory(&self.location.install_dir)?,
            shims_dir: plain_directory(&self.location.shims_dir)?,
            before,
            after: after.clone(),
            phase: Phase::Pending,
        };
        journal.validate(&identity.sid, &identity.target)?;
        // Persist bounded intent before touching HKLM; pending exact-after state is recoverable.
        // 修改 HKLM 前持久化有界意图；未完成标记但精确匹配 after 的状态仍可恢复。
        self.write_store_journal(&journal)?;
        self.verify_machine_installation_path()?;
        publish_path(&key, original.as_ref(), &after, &identity.target, false)?;
        journal.phase = Phase::Applied;
        self.write_store_journal(&journal)?;
        Ok(true)
    }

    fn finish_restored_entry(&self, identity: &Identity) -> Result<bool> {
        let _directory = DirectoryLease::acquire(&self.location.config_dir, false)?;
        let _lock = FileLease::acquire(
            &self.location.config_dir.join("store-path.lock"),
            true,
            true,
        )?;
        let Some(mut journal) = self.read_store_journal(identity)? else {
            return Ok(true);
        };
        let current = decoded(read_machine_path()?.as_ref())?;
        if journal.restoration(current.as_ref())?.is_some() {
            return Ok(false);
        }
        journal.phase = Phase::Restored;
        self.write_store_journal(&journal)?;
        Ok(true)
    }

    fn restore_store_entry(&self, identity: &Identity) -> Result<bool> {
        let Some(existing) = self.read_store_journal(identity)? else {
            return Ok(false);
        };
        if existing.phase == Phase::Restored {
            return Ok(false);
        }
        let _directory = DirectoryLease::acquire(&self.location.config_dir, false)?;
        let _lock = FileLease::acquire(
            &self.location.config_dir.join("store-path.lock"),
            true,
            true,
        )?;
        let Some(mut journal) = self.read_store_journal(identity)? else {
            return Ok(false);
        };
        let original = read_machine_path()?;
        let current = decoded(original.as_ref())?;
        let desired = journal.restoration(current.as_ref())?;
        if let Some(next) = &desired {
            let key = machine_key(true)?;
            // Only the fixed ProfileList-derived target is inserted; never restore another user's PATH.
            // 仅插入由 ProfileList 派生的固定目标，绝不恢复其他用户的 PATH。
            publish_path(&key, original.as_ref(), next, &identity.target, true)?;
        }
        journal.phase = Phase::Restored;
        self.write_store_journal(&journal)?;
        Ok(desired.is_some())
    }
}

struct Identity {
    sid: String,
    target: String,
}

impl Identity {
    fn read(sid: &str) -> Result<Self> {
        validate_sid(sid)?;
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(format!(r"{PROFILES}\{sid}"), KEY_READ | KEY_WOW64_64KEY)
            .map_err(|_| usage("Cannot locate the original user's machine profile record"))?;
        let value = key
            .get_raw_value("ProfileImagePath")
            .map_err(|_| usage("Cannot read the original user's machine profile directory"))?;
        let profile = decode_value(&value)?.text;
        // Do not expand environment variables from the elevated account or accept redirected paths.
        // 不展开提升账户的环境变量，也不接受重定向路径。
        validate_profile(&profile)?;
        Ok(Self {
            sid: sid.to_owned(),
            target: format!(
                r"{}\AppData\Local\Microsoft\WindowsApps",
                profile.trim_end_matches('\\')
            ),
        })
    }
}

fn validate_sid(sid: &str) -> Result<()> {
    let parts: Vec<_> = sid.split('-').collect();
    if sid.len() > 184
        || !(4..=18).contains(&parts.len())
        || parts.first() != Some(&"S")
        || parts.get(1) != Some(&"1")
        || parts.iter().skip(2).any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || part.parse::<u64>().is_err()
        })
    {
        return Err(usage("Invalid original-user SID"));
    }
    Ok(())
}

fn validate_profile(profile: &str) -> Result<()> {
    let bytes = profile.as_bytes();
    if bytes.len() < 4
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1..3] != *b":\\"
        || profile.contains(['%', ';', '"', '/', '\0', '\r', '\n'])
        || profile[3..]
            .split('\\')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(usage(
            "ProfileImagePath must be a fixed local directory; refusing environment-derived or redirected targets",
        ));
    }
    Ok(())
}

fn plain_directory(path: &Path) -> Result<String> {
    let text = path.to_str().ok_or_else(|| usage("PATH must be Unicode"))?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    validate_profile(text)?;
    Ok(text.to_owned())
}

fn machine_key(write: bool) -> Result<RegKey> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            MACHINE_ENVIRONMENT,
            KEY_READ | KEY_WOW64_64KEY | if write { KEY_SET_VALUE } else { 0 },
        )
        .map_err(|_| {
            Error::new(
                ErrorKind::Permission,
                "Cannot open machine PATH; administrator consent is required only for this repair",
            )
        })
}

fn read_machine_path() -> Result<Option<RegValue>> {
    read_raw_path(&machine_key(false)?)
}

fn read_raw_path(key: &RegKey) -> Result<Option<RegValue>> {
    match key.get_raw_value("Path") {
        Ok(value) => {
            decode_value(&value)?;
            Ok(Some(value))
        }
        Err(error) if error.kind() == IoErrorKind::NotFound => Ok(None),
        Err(_) => Err(usage("Cannot read PATH for Windows Store conflict repair")),
    }
}

fn decoded(value: Option<&RegValue>) -> Result<Option<PathValue>> {
    value.map(decode_value).transpose()
}

fn decode_value(value: &RegValue) -> Result<PathValue> {
    let kind = match value.vtype {
        REG_SZ => ValueKind::String,
        REG_EXPAND_SZ => ValueKind::ExpandString,
        _ => return Err(usage("PATH or profile has an unsupported registry type")),
    };
    if value.bytes.len() > 65_534 || value.bytes.len() % 2 != 0 {
        return Err(usage("PATH or profile has invalid UTF-16 data"));
    }
    let units: Vec<_> = value
        .bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).map_err(|_| usage("PATH or profile is not Unicode"))?;
    let value = PathValue {
        kind,
        text: text.trim_end_matches('\0').to_owned(),
    };
    value.validate()?;
    Ok(value)
}

fn publish_path(
    key: &RegKey,
    expected: Option<&RegValue>,
    next: &PathValue,
    target: &str,
    restore: bool,
) -> Result<()> {
    next.validate()?;
    let current = decoded(expected)?;
    let unrelated = current.as_ref().map_or_else(
        || PathValue {
            kind: next.kind,
            text: String::new(),
        },
        |value| value.without(target),
    );
    // Recheck the actual write delta, not merely the user-writable journal's claimed snapshots.
    // 再次核验真实写入差异，不只检查用户可写日志所声称的快照。
    if next.without(target) != unrelated
        || next.contains(target) != restore
        || current
            .as_ref()
            .is_some_and(|value| value.kind != next.kind)
    {
        return Err(usage(
            "Store PATH repair would modify unrelated machine entries",
        ));
    }
    if read_raw_path(key)?.as_ref() != expected {
        return Err(usage("Machine PATH changed concurrently; retry the repair"));
    }
    key.set_raw_value(
        "Path",
        &RegValue {
            bytes: next
                .text
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
            vtype: match next.kind {
                ValueKind::String => REG_SZ,
                ValueKind::ExpandString => REG_EXPAND_SZ,
            },
        },
    )
    .map_err(|_| usage("Cannot publish Windows Store PATH repair"))?;
    notify_environment_change();
    Ok(())
}

fn report(changed: bool, restore: bool) -> Value {
    json!({
        "system_store_path_changed": changed,
        "restored": restore,
        "restart_terminal": changed,
        "scope": "duplicated_owner_windowsapps_only"
    })
}
