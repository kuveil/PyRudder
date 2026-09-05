//! Explicit machine PATH publication with native elevation and scoped ownership.
//! 通过原生权限提升及有限所有权记录显式发布系统 PATH。

use super::{
    App,
    path::{decode_path, path_text, read_path, same_entry},
    usage,
};
use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    elevation,
    environment::notify_environment_change,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease},
};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::Path};
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_EXPAND_SZ},
};

const JOURNAL: &str = "system-path.json";
const ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    schema_version: u32,
    scope: String,
    install_dir: String,
    shims_dir: String,
    added: Vec<String>,
}

impl App {
    pub(super) fn update_system_path(&self, add: bool, elevated: bool) -> Result<bool> {
        let targets = [
            path_text(&self.location.install_dir)?,
            path_text(&self.location.shims_dir)?,
        ];
        if elevated {
            return self.publish_system_path(add, &targets);
        }
        // A manual-PATH installation has nothing to undo and must not request elevation.
        // 手动配置 PATH 的安装没有可撤销项，不应请求权限提升。
        if !add
            && self
                .system_path_ownership(&targets)?
                .is_none_or(|ownership| ownership.added.is_empty())
            && !self.has_store_path_ownership()?
        {
            return Ok(false);
        }
        let before = read_path(&machine_key(false)?)?;
        let owner_sid = elevation::current_user_sid()?;
        let program =
            std::env::current_exe().map_err(|_| usage("Cannot locate the installed CLI"))?;
        // One elevated helper updates machine PATH and the exact original user's Store conflict.
        // 一个提升后的辅助进程同时更新系统 PATH 和原账户的精确商店冲突项。
        let exit =
            elevation::run_elevated(&program, &self.system_path_arguments(&owner_sid, !add)?)?;
        if exit != 0 {
            return Err(Error::new(
                ErrorKind::Permission,
                format!("System PATH update failed with exit code {exit}"),
            )
            .with_hint("Administrator permission is required. Rerun the installer or uninstaller; no terminal initialization command is needed."));
        }
        let after = read_path(&machine_key(false)?)?;
        if add {
            let text = decode_path(after.as_ref())?;
            let entries: Vec<_> = text.split(';').collect();
            if targets
                .iter()
                .any(|target| !entries.iter().any(|entry| same_entry(entry, target)))
            {
                return Err(usage("System PATH changed again; rerun the installer"));
            }
        }
        Ok(before != after)
    }

    fn system_path_arguments(&self, owner_sid: &str, remove: bool) -> Result<Vec<OsString>> {
        let paths = &self.configuration.paths;
        let home = paths
            .install_dir
            .parent()
            .ok_or_else(|| usage("CLI installation has no parent directory"))?;
        let mut arguments = Vec::new();
        // Fix every location so elevation under another account cannot select another installation.
        // 固定全部路径，避免使用其他账户提升权限时误选另一安装。
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
            OsString::from("__system-path"),
            OsString::from("--owner-sid"),
            OsString::from(owner_sid),
        ]);
        if remove {
            arguments.push(OsString::from("--remove"));
        }
        Ok(arguments)
    }

    fn system_path_ownership(&self, targets: &[String; 2]) -> Result<Option<Ownership>> {
        let Some(bytes) =
            WindowsStateFileSystem.read_file(&self.location.config_dir.join(JOURNAL), 65_536)?
        else {
            return Ok(None);
        };
        let ownership: Ownership = serde_json::from_slice(&bytes)
            .map_err(|_| usage("Invalid system PATH ownership journal"))?;
        if ownership.schema_version != 1
            || ownership.scope != "system"
            || ownership.install_dir != targets[0]
            || ownership.shims_dir != targets[1]
            || ownership.added.len() > 2
            || ownership.added.iter().any(|entry| !targets.contains(entry))
        {
            return Err(usage("System PATH journal belongs to another installation"));
        }
        Ok(Some(ownership))
    }

    fn publish_system_path(&self, add: bool, targets: &[String; 2]) -> Result<bool> {
        let _directory = DirectoryLease::acquire(&self.location.config_dir, false)?;
        let _lock = FileLease::acquire(
            &self.location.config_dir.join("system-path.lock"),
            true,
            true,
        )?;
        let previous = self
            .system_path_ownership(targets)?
            .unwrap_or_else(|| Ownership {
                schema_version: 1,
                scope: "system".into(),
                install_dir: targets[0].clone(),
                shims_dir: targets[1].clone(),
                added: Vec::new(),
            });
        if !add && previous.added.is_empty() {
            return Ok(false);
        }
        let key = machine_key(true)?;
        let original = read_path(&key)?;
        let text = decode_path(original.as_ref())?;
        let mut entries: Vec<_> = if text.is_empty() {
            Vec::new()
        } else {
            text.split(';').map(str::to_owned).collect()
        };
        let mut ownership = previous.clone();
        if add {
            for target in targets {
                if !entries.iter().any(|entry| same_entry(entry, target))
                    && !ownership.added.contains(target)
                {
                    ownership.added.push(target.clone());
                }
            }
            // Existing targets are moved but never claimed; append after all unrelated system entries.
            // 已有目标只移动、不获取所有权；追加在所有无关系统项之后。
            entries.retain(|entry| !targets.iter().any(|target| same_entry(entry, target)));
            entries.extend(targets.iter().cloned());
        } else {
            entries.retain(|entry| !ownership.added.iter().any(|owned| same_entry(entry, owned)));
        }
        let next = entries.join(";");
        if next.encode_utf16().count() >= 32_766 {
            return Err(usage("Resulting system PATH exceeds the Windows limit"));
        }
        let journal = self.location.config_dir.join(JOURNAL);
        if add {
            // Record only intended new entries before publication, so interruption is recoverable.
            // 发布前仅记录计划新增的项，确保中断后仍可清理。
            write_ownership(&journal, &ownership)?;
        }
        if next != text {
            if let Err(error) = write_machine_path(&key, original.as_ref(), &next) {
                if add && write_ownership(&journal, &previous).is_err() {
                    return Err(error.with_hint("PATH update failed and the ownership journal could not be rolled back; keep this installation for recovery."));
                }
                return Err(error);
            }
        }
        if !add {
            ownership.added.clear();
            write_ownership(&journal, &ownership)?;
        }
        Ok(next != text)
    }
}

fn machine_key(write: bool) -> Result<RegKey> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            ENVIRONMENT,
            KEY_READ | KEY_WOW64_64KEY | if write { KEY_SET_VALUE } else { 0 },
        )
        .map_err(|_| {
            Error::new(
                ErrorKind::Permission,
                "Cannot open system PATH; administrator permission is required",
            )
        })
}

fn write_ownership(path: &Path, ownership: &Ownership) -> Result<()> {
    let bytes = serde_json::to_vec(ownership)
        .map_err(|_| usage("Cannot encode system PATH ownership journal"))?;
    if bytes.len() > 65_536 {
        return Err(usage(
            "System PATH ownership journal exceeds the size limit",
        ));
    }
    WindowsStateFileSystem.write_atomic(path, &bytes)
}

fn write_machine_path(key: &RegKey, expected: Option<&RegValue>, next: &str) -> Result<()> {
    if read_path(key)?.as_ref() != expected {
        return Err(usage("System PATH changed concurrently; retry the command"));
    }
    key.set_raw_value(
        "Path",
        &RegValue {
            bytes: next
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
            vtype: expected.map_or(REG_EXPAND_SZ, |value| value.vtype.clone()),
        },
    )
    .map_err(|_| Error::new(ErrorKind::Permission, "Cannot update system PATH"))?;
    notify_environment_change();
    Ok(())
}
