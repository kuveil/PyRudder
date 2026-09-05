//! Bounded user configuration editing with backups and explicit path-change boundaries.
//! 带备份和显式路径变更边界的有界用户配置编辑。

use super::{App, usage};
use crate::arguments::ConfigAction;
use pyrudder_core::{Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    registry::Snapshot, state::WindowsStateFileSystem, storage::DirectoryLease,
};
use serde_json::{Value, json};
use std::fmt::Write;

impl App {
    pub(super) fn config(&self, action: &ConfigAction) -> Result<Value> {
        let path = self.location.config_dir.join("config.toml");
        let bytes = WindowsStateFileSystem.read_file(&path, 65_536)?;
        let mut document: toml::Value = if let Some(bytes) = &bytes {
            toml::from_str(
                std::str::from_utf8(bytes).map_err(|_| usage("Configuration must be UTF-8"))?,
            )
            .map_err(|_| usage("Invalid configuration TOML"))?
        } else {
            toml::from_str("schema_version = 1\n")
                .map_err(|_| usage("Invalid default configuration"))?
        };
        let (key, value) = match action {
            ConfigAction::Get { key } => {
                let result = if let Some(key) = key {
                    let (section, key) = config_key(key)?;
                    document
                        .get(section)
                        .and_then(|section| section.get(key))
                        .cloned()
                        .unwrap_or_else(|| {
                            toml::Value::String("<default or environment override>".into())
                        })
                } else {
                    document
                };
                return Ok(
                    json!({"file": path, "value": result, "effective_paths": self.paths_json()}),
                );
            }
            ConfigAction::Set { key, value } => (key, Some(value)),
            ConfigAction::Unset { key } => (key, None),
        };
        let (section, item) = config_key(key)?;
        let transaction = self.registry.transaction()?;
        check_path_change(section, item, &transaction.snapshot)?;
        let value = value
            .map(|value| {
                if section == "commands" {
                    if value != "false" {
                        return Err(usage(
                            "System fallback is not supported; use false / 不支持系统回退，请使用 false",
                        ));
                    }
                    Ok(toml::Value::Boolean(false))
                } else {
                    let path =
                        WindowsStateFileSystem.normalize_directory(std::path::Path::new(value))?;
                    let key = pyrudder_platform_windows::storage::path_key(&path)?;
                    let effective = self.paths_json();
                    for (other, directory) in effective.as_object().ok_or_else(|| usage("Missing effective path map"))? {
                        if other == item { continue; }
                        let directory = directory.as_str().ok_or_else(|| usage("Invalid effective path"))?;
                        let other_key = pyrudder_platform_windows::storage::path_key(std::path::Path::new(directory))?;
                        if key.starts_with(&other_key) || other_key.starts_with(&key) {
                            return Err(usage("Configuration directories must not overlap; existing file was preserved"));
                        }
                    }
                    Ok(toml::Value::String(
                        path.to_str()
                            .ok_or_else(|| usage("Configuration paths must be Unicode"))?
                            .to_owned(),
                    ))
                }
            })
            .transpose()?;
        let table = document
            .as_table_mut()
            .ok_or_else(|| usage("Configuration root must be a table"))?;
        let section = table
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| usage("Configuration section must be a table"))?;
        if let Some(value) = value {
            section.insert(item.into(), value);
        } else {
            section.remove(item);
        }
        let output =
            toml::to_string_pretty(&document).map_err(|_| usage("Cannot encode configuration"))?;
        if output.len() > 65_536 {
            return Err(usage("Configuration exceeds 65536 bytes"));
        }
        let _anchor = DirectoryLease::acquire(&self.location.config_dir, true)?;
        if let Some(bytes) = bytes {
            WindowsStateFileSystem
                .write_atomic(&self.location.config_dir.join("config.toml.backup"), &bytes)?;
        }
        WindowsStateFileSystem.write_atomic(&path, output.as_bytes())?;
        Ok(
            json!({"updated": key, "file": path, "message": "Configuration updated; existing runtime files were not moved."}),
        )
    }

    pub(super) fn paths_json(&self) -> Value {
        let paths = &self.configuration.paths;
        json!({"install_dir": paths.install_dir, "shims_dir": paths.shims_dir, "config_dir": paths.config_dir,
            "runtimes_dir": paths.runtimes_dir, "downloads_dir": paths.downloads_dir, "cache_dir": paths.cache_dir, "temp_dir": paths.temp_dir})
    }

    pub(super) fn initial_config(&self) -> Result<String> {
        let paths = self.paths_json();
        let mut document = String::from("schema_version = 1\n\n[paths]\n");
        for key in [
            "install_dir",
            "shims_dir",
            "runtimes_dir",
            "downloads_dir",
            "cache_dir",
            "temp_dir",
        ] {
            let value = paths
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| usage("Missing setup path"))?;
            let quoted =
                serde_json::to_string(value).map_err(|_| usage("Cannot quote setup path"))?;
            writeln!(document, "{key} = {quoted}")
                .map_err(|_| usage("Cannot format setup paths"))?;
        }
        document.push_str("\n[commands]\nsystem_fallback = false\n");
        Ok(document)
    }
}

// Preserve owned program/runtime roots; configuring unused storage needs no migration.
// 保留已拥有的程序/运行时根目录；尚未使用的存储目录可直接配置。
fn check_path_change(section: &str, item: &str, snapshot: &Snapshot) -> Result<()> {
    if section != "paths" {
        return Ok(());
    }
    if matches!(item, "install_dir" | "shims_dir") && snapshot.generation != 0 {
        return Err(usage(
            "Changing program/shim roots requires migration; use a separate installation directory",
        ));
    }
    if item == "runtimes_dir"
        && snapshot.runtimes.iter().any(|runtime| {
            matches!(
                runtime.origin(),
                pyrudder_core::runtime::RuntimeOrigin::Managed { .. }
            )
        })
    {
        return Err(usage(
            "Managed Python already exists. Its root is protected; use --runtimes-dir for an explicit new installation. Existing Python is never moved automatically.",
        ));
    }
    Ok(())
}

fn config_key(key: &str) -> Result<(&str, &str)> {
    let (section, item) = key.split_once('.').unwrap_or(("paths", key));
    if (section == "paths"
        && matches!(
            item,
            "install_dir"
                | "shims_dir"
                | "runtimes_dir"
                | "downloads_dir"
                | "cache_dir"
                | "temp_dir"
        ))
        || (section == "commands" && item == "system_fallback")
    {
        Ok((section, item))
    } else {
        Err(usage(
            "Unknown configuration key; config_dir is chosen outside its own file",
        ))
    }
}
