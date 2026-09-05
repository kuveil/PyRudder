//! Interactive official release selection with a one-install destination override.
//! 交互选择官方版本，安装目录覆盖仅对本次安装生效。

use super::{App, Outcome, path::path_text, usage};
use crate::prompts;
use pyrudder_core::{
    Result,
    runtime::{RuntimeHealth, RuntimeId, RuntimeRecord},
    state::StateFileSystem,
};
use pyrudder_platform_windows::state::WindowsStateFileSystem;
use pyrudder_provider_pythonorg::Release;
use serde_json::json;
use std::path::{Path, PathBuf};

impl App {
    pub(super) fn available(&self, offline: bool, list: bool) -> Result<Outcome> {
        let releases = self.releases(offline)?;
        // Redirected output, JSON and quiet mode never start an installation.
        // 重定向输出、JSON 和安静模式均不会触发安装。
        if list || !self.interactive || releases.is_empty() {
            return Ok(Outcome::Data(json!({
                "provider": "python.org", "signature_required": true,
                "offline": offline, "releases": releases,
            })));
        }
        let snapshot = self.registry.load()?;
        let ids = releases
            .iter()
            .map(|release| RuntimeId::new(release.version.parse()?))
            .collect::<Result<Vec<_>>>()?;
        let items = releases
            .iter()
            .zip(&ids)
            .map(
                |(release, id)| match snapshot.runtimes.iter().find(|record| record.id() == id) {
                    Some(record) if matches!(record.health(), RuntimeHealth::Ready) => {
                        format!("Python {}  [已安装 / installed]", release.version)
                    }
                    Some(_) => format!("Python {}  [未完成 / pending]", release.version),
                    None => format!("Python {}", release.version),
                },
            )
            .collect::<Vec<_>>();
        let Some(index) = prompts::choose_version(&items)? else {
            return Ok(cancelled());
        };
        let id = &ids[index];
        self.install_selected(
            &releases[index],
            id,
            snapshot.runtimes.iter().find(|record| record.id() == id),
            offline,
        )
    }

    fn install_selected(
        &self,
        release: &Release,
        id: &RuntimeId,
        existing: Option<&RuntimeRecord>,
        offline: bool,
    ) -> Result<Outcome> {
        let directory = if let Some(record) = existing {
            if matches!(record.health(), RuntimeHealth::Ready) {
                return Ok(Outcome::Text(format!(
                    "Python {} 已安装 / already installed: {}\npyrudder global {id}",
                    release.version,
                    path_text(record.root())?,
                )));
            }
            if matches!(record.health(), RuntimeHealth::PendingRemoval) {
                return Err(usage(
                    "请先完成该版本的卸载，再重新安装 / Finish the pending uninstall before reinstalling",
                ));
            }
            // Resume in the owned location instead of silently ignoring a newly entered path.
            // 从已有位置继续，避免静默忽略用户新输入的目录。
            eprintln!(
                "继续未完成的安装，保留原目录 / Resuming installation in its original directory: {}",
                path_text(record.root())?
            );
            None
        } else {
            let Some(directory) = self.new_runtime_directory(id)? else {
                return Ok(cancelled());
            };
            Some(directory)
        };
        let result = self.install_release(release, None, offline, directory.as_deref())?;
        let root = result["installed"]["root"]
            .as_str()
            .or_else(|| result["already_installed"]["root"].as_str())
            .map(PathBuf::from)
            .ok_or_else(|| usage("Installation returned no runtime path"))?;
        Ok(Outcome::Text(format!(
            "Python {} 安装完成 / installed: {}\n使用以下命令设为全局版本 / To select globally:\npyrudder global {id}",
            release.version,
            path_text(&root)?,
        )))
    }

    fn new_runtime_directory(&self, id: &RuntimeId) -> Result<Option<PathBuf>> {
        // Enter always chooses this PyRudder installation, not a previous custom override.
        // 直接回车始终选择当前 PyRudder 安装目录，不沿用以前的自定义覆盖。
        let default = self
            .location
            .install_dir
            .parent()
            .ok_or_else(|| usage("PyRudder installation has no parent directory"))?
            .join("runtimes");
        eprintln!(
            "Python {}；每个版本将安装到独立子目录 / Each version uses its own subdirectory: {id}",
            id.version()
        );
        let displayed_default = PathBuf::from(path_text(&default)?);
        prompts::installation_directory(&displayed_default, |input| {
            self.installation_destination(input, &default)
        })
    }

    fn installation_destination(&self, input: &str, default: &Path) -> Result<PathBuf> {
        let path = destination_path(input, default)?;
        self.managed_install_directory(&path)
    }
}

fn destination_path(input: &str, default: &Path) -> Result<PathBuf> {
    let input = input.trim();
    let input = input
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(input);
    let path = if input.is_empty() {
        default
    } else {
        Path::new(input)
    };
    if !path.is_absolute() || path.parent().is_none() {
        return Err(usage(
            "请输入本地绝对目录，例如 D:\\Python，或直接回车 / Enter an absolute local directory or press Enter",
        ));
    }
    path_text(path)?;
    WindowsStateFileSystem.normalize_directory(path)
}

fn cancelled() -> Outcome {
    Outcome::Text("已取消，未安装 Python。 / Cancelled; no Python was installed.".into())
}

#[cfg(test)]
mod tests {
    use super::destination_path;
    use pyrudder_core::Result;
    use pyrudder_core::state::StateFileSystem;
    use pyrudder_platform_windows::state::WindowsStateFileSystem;
    use pyrudder_platform_windows::storage::path_key;

    #[test]
    fn enter_and_quoted_unicode_paths_are_supported_without_creating_them() -> Result<()> {
        let directory = tempfile::tempdir().map_err(|error| super::usage(&error.to_string()))?;
        let directory = WindowsStateFileSystem.normalize_directory(directory.path())?;
        let default = directory.join("default");
        assert_eq!(
            path_key(&destination_path("", &default)?)?,
            path_key(&default)?
        );
        let custom = directory.join("开发 Python");
        assert_eq!(
            path_key(&destination_path(
                &format!("\"{}\"", custom.display()),
                &default
            )?)?,
            path_key(&custom)?
        );
        assert!(!default.exists());
        assert!(!custom.exists());
        Ok(())
    }

    #[test]
    fn relative_root_and_unsafe_paths_are_rejected() {
        let default = std::path::Path::new(r"C:\PyRudder\runtimes");
        for input in [
            r"relative",
            r"C:\",
            r"\\server\share\Python",
            r"C:\Python;bad",
            r"C:\%USERPROFILE%\Python",
        ] {
            assert!(destination_path(input, default).is_err(), "{input}");
        }
    }
}
