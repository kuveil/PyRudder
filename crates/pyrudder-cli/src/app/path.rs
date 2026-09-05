//! Reversible per-installation user PATH changes with an ownership journal.
//! 使用所有权日志实现可撤销的单安装用户 PATH 修改。

use super::{App, usage};
use crate::arguments::PathAction;
use pyrudder_core::{Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    environment::notify_environment_change, state::WindowsStateFileSystem, storage::FileLease,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::ErrorKind, path::Path};
use winreg::{
    RegKey, RegValue,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PathOwnership {
    schema_version: u32,
    added: Vec<String>,
}

impl App {
    pub(super) fn path(&self, action: &PathAction) -> Result<Value> {
        match action {
            PathAction::Add => Ok(
                json!({"system_path_changed": self.update_system_path(true, false)?, "restart_terminal": true}),
            ),
            PathAction::Remove => {
                // Remove current machine ownership, then clean older user-PATH ownership if present.
                // 先移除当前系统所有权，再按需清理旧版用户 PATH 所有权。
                let machine = self.update_system_path(false, false)?;
                let changed = self.update_user_path(false)?;
                Ok(
                    json!({"legacy_user_path_changed": changed, "system_path_changed": machine, "restart_terminal": true}),
                )
            }
        }
    }

    pub(super) fn update_user_path(&self, add: bool) -> Result<bool> {
        let journal = self.location.config_dir.join("user-path.json");
        // Removal without a journal is a no-op, including uninstall after interrupted setup.
        // 无日志时移除操作不做修改，也适用于初始化中断后的卸载。
        if !add
            && WindowsStateFileSystem
                .read_file(&journal, 65_536)?
                .is_none()
        {
            return Ok(false);
        }
        let _lock =
            FileLease::acquire(&self.location.config_dir.join("user-path.lock"), true, true)?;
        let paths = [
            path_text(&self.location.install_dir)?,
            path_text(&self.location.shims_dir)?,
        ];
        let mut ownership = match WindowsStateFileSystem.read_file(&journal, 65_536)? {
            Some(bytes) => serde_json::from_slice::<PathOwnership>(&bytes)
                .map_err(|_| usage("Invalid user PATH ownership journal"))?,
            None => PathOwnership {
                schema_version: 1,
                added: Vec::new(),
            },
        };
        if ownership.schema_version != 1
            || ownership.added.len() > 2
            || ownership.added.iter().any(|entry| !paths.contains(entry))
        {
            return Err(usage(
                "PATH journal belongs to another installation; refusing removal",
            ));
        }
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
            .map_err(|_| usage("Cannot open current-user environment registry key"))?
            .0;
        let original = read_path(&key)?;
        let text = decode_path(original.as_ref())?;
        let mut entries: Vec<String> = if text.is_empty() {
            Vec::new()
        } else {
            text.split(';').map(str::to_owned).collect()
        };
        if add {
            for path in paths.iter().rev() {
                if !entries.iter().any(|entry| same_entry(entry, path)) {
                    entries.insert(0, path.clone());
                    if !ownership.added.contains(path) {
                        ownership.added.push(path.clone());
                    }
                }
            }
        } else {
            entries.retain(|entry| !ownership.added.iter().any(|owned| same_entry(entry, owned)));
        }
        let next = entries.join(";");
        if next.encode_utf16().count() >= 32_766 {
            return Err(usage("Resulting user PATH exceeds the Windows limit"));
        }
        if add {
            // Persist intent first, so interrupted PATH publication can be retried or undone.
            // 先持久化意图，PATH 写入中断后可以重试或撤销。
            write_ownership(&journal, &ownership)?;
        }
        if next != text {
            if read_path(&key)? != original {
                return Err(usage("User PATH changed concurrently; retry the command"));
            }
            key.set_raw_value(
                "Path",
                &RegValue {
                    bytes: next
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .flat_map(u16::to_le_bytes)
                        .collect(),
                    vtype: original
                        .as_ref()
                        .map_or(REG_EXPAND_SZ, |value| value.vtype.clone()),
                },
            )
            .map_err(|_| usage("Cannot update user PATH"))?;
            notify_environment_change();
        }
        if !add {
            ownership.added.clear();
            write_ownership(&journal, &ownership)?;
        }
        Ok(next != text)
    }
}

fn write_ownership(path: &Path, ownership: &PathOwnership) -> Result<()> {
    let bytes = serde_json::to_vec(ownership).map_err(|_| usage("Cannot encode PATH journal"))?;
    WindowsStateFileSystem.write_atomic(path, &bytes)
}

pub(super) fn read_path(key: &RegKey) -> Result<Option<RegValue>> {
    match key.get_raw_value("Path") {
        Ok(value) if matches!(value.vtype, REG_SZ | REG_EXPAND_SZ) => Ok(Some(value)),
        Ok(_) => Err(usage("PATH has an unsupported registry value type")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(usage("Cannot read PATH")),
    }
}

pub(super) fn decode_path(value: Option<&RegValue>) -> Result<String> {
    let Some(value) = value else {
        return Ok(String::new());
    };
    if value.bytes.len() % 2 != 0 || value.bytes.len() > 65_534 {
        return Err(usage(
            "PATH has invalid UTF-16 or exceeds the Windows limit",
        ));
    }
    let units: Vec<_> = value
        .bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let text = String::from_utf16(&units).map_err(|_| usage("PATH is not valid Unicode"))?;
    let text = text.trim_end_matches('\0');
    if text.contains('\0') {
        return Err(usage("PATH contains an embedded NUL"));
    }
    Ok(text.to_owned())
}

pub(super) fn path_text(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| usage("PATH directory must be Unicode"))?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    if text.contains([';', '"', '\r', '\n', '%']) {
        return Err(usage("Cannot safely persist this PATH directory"));
    }
    Ok(text.to_owned())
}

pub(super) fn same_entry(left: &str, right: &str) -> bool {
    left.strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(left)
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(right.trim_end_matches('\\'))
}
