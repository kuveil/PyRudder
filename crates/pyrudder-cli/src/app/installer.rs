//! Internal setup entry point with a fixed identity and upgrade eligibility checks.
//! 身份固定并校验升级条件的内部安装入口。

use super::{App, Outcome, installer_transaction, setup, usage};
use crate::arguments::{Arguments, InstallerPhase};
use pyrudder_core::{Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    registry::Location,
    router,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, path_key},
};
use serde_json::json;
use std::path::Path;

pub(crate) fn run(
    arguments: &Arguments,
    root: &Path,
    phase: InstallerPhase,
    from_version: Option<&str>,
) -> Result<Outcome> {
    let root = WindowsStateFileSystem.normalize_directory(root)?;
    if root.parent().is_none() || root.components().count() < 2 {
        return Err(usage(
            "Installer requires a non-root local directory / 安装器必须使用非盘符根目录",
        ));
    }
    // Recovery must not depend on the possibly half-replaced binaries or configuration.
    // 恢复不能依赖可能只替换了一半的程序或配置。
    match phase {
        InstallerPhase::Rollback => {
            let previous = installer_transaction::recover(&root)?;
            return Ok(Outcome::Text(previous.map_or_else(
                || "PYRUDDER_RECOVERY_UNCHANGED".to_owned(),
                |version| format!("PYRUDDER_RESTORED_VERSION={version}"),
            )));
        }
        InstallerPhase::RecoveryComplete => {
            installer_transaction::acknowledge_recovery(&root)?;
            return Ok(Outcome::Data(json!({"recovery_complete": true})));
        }
        InstallerPhase::Commit => {
            installer_transaction::commit(&root)?;
            return Ok(Outcome::Data(json!({"committed": true})));
        }
        _ => {}
    }
    let application = App::load(arguments)?;
    validate_layout(&root, &application.location)?;
    match phase {
        InstallerPhase::Check | InstallerPhase::Prepare => {
            check_installation(&root, &application, from_version)?;
            if matches!(phase, InstallerPhase::Prepare) {
                let previous = from_version.ok_or_else(|| {
                    usage("Upgrade requires the installed version / 更新必须指定原版本")
                })?;
                installer_transaction::prepare(&root, &application.location, previous)?;
            }
            Ok(Outcome::Data(
                json!({"checked": true, "upgrade": from_version.is_some(), "version": env!("CARGO_PKG_VERSION")}),
            ))
        }
        InstallerPhase::Finish => {
            let _anchor = DirectoryLease::acquire(&root, false)?;
            if path_key(&router::current_program_directory()?)? != path_key(&root.join("bin"))? {
                return Err(usage(
                    "Setup finalization must run from the installed bin directory / 安装完成步骤必须从安装目录的 bin 运行",
                ));
            }
            let _exclusive =
                FileLease::acquire(&root.join("config").join("installer.lock"), true, true)?;
            application.setup(false)
        }
        InstallerPhase::Commit | InstallerPhase::Rollback | InstallerPhase::RecoveryComplete => {
            unreachable!()
        }
    }
}

fn validate_layout(root: &Path, location: &Location) -> Result<()> {
    for (actual, suffix) in [
        (&location.install_dir, "bin"),
        (&location.config_dir, "config"),
        (&location.shims_dir, "shims"),
    ] {
        let expected = WindowsStateFileSystem.normalize_directory(&root.join(suffix))?;
        if path_key(actual)? != path_key(&expected)? {
            return Err(usage(
                "Installer-managed bin/config/shims must remain under the original root; data directories may be customized. / 安装器管理的 bin/config/shims 必须保留在原根目录，可自定义 Python 等数据目录。",
            ));
        }
    }
    Ok(())
}

fn check_installation(root: &Path, application: &App, from_version: Option<&str>) -> Result<()> {
    let Some(previous) = from_version else {
        if root
            .try_exists()
            .map_err(|_| usage("Cannot inspect setup directory"))?
        {
            let _anchor = DirectoryLease::acquire(root, false)?;
            if std::fs::read_dir(root)
                .map_err(|_| usage("Cannot inspect setup directory"))?
                .next()
                .is_some()
            {
                return Err(usage(
                    "Fresh setup requires an empty directory / 首次安装需要空目录",
                ));
            }
        }
        return Ok(());
    };
    validate_versions(previous, env!("CARGO_PKG_VERSION"))?;
    let _anchor = DirectoryLease::acquire(root, false)?;
    let _exclusive = FileLease::acquire(&root.join("config").join("installer.lock"), true, false)?;
    let bytes = WindowsStateFileSystem
        .read_file(&root.join("BUILD-INFO.json"), 65_536)?
        .ok_or_else(|| usage("Missing installed build identity / 缺少已安装版本信息"))?;
    let build: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| usage("Invalid installed build identity / 已安装版本信息损坏"))?;
    if build["name"] != "PyRudder" || build["version"] != previous {
        return Err(usage(
            "Installed version does not match its registration; restore the interrupted update first. / 程序版本与安装记录不符，请先恢复中断的更新。",
        ));
    }
    if setup::package_home(&root.join("bin"))?.is_none() {
        return Err(usage("Missing package layout marker / 缺少程序布局标记"));
    }
    for directory in [
        &application.location.install_dir,
        &application.location.config_dir,
        &application.location.shims_dir,
    ] {
        if Location::read(directory)? != application.location {
            return Err(usage(
                "Installed location contract differs / 安装位置记录不一致",
            ));
        }
    }
    if application.configuration.loaded_file.is_none()
        || WindowsStateFileSystem
            .read_file(&root.join("config").join("registry.json"), 32 * 1024 * 1024)?
            .is_none()
    {
        return Err(usage(
            "Missing installed state; refusing to replace it with empty state. / 已安装状态缺失，拒绝用空状态替代。",
        ));
    }
    // Preparation validates ownership together with interrupted publication records.
    // 准备阶段结合未完成的发布记录统一校验文件所有权。
    application.registry.load()?;
    Ok(())
}

fn stable_version(text: &str) -> Result<[u32; 3]> {
    let components: Vec<_> = text.split('.').collect();
    if components.len() != 3 {
        return Err(usage(
            "In-place updates require stable versions from 0.1.0 onward; uninstall Alpha first. / 原地更新从 0.1.0 正式版本起支持，请先卸载 Alpha。",
        ));
    }
    let mut version = [0; 3];
    for (output, component) in version.iter_mut().zip(components) {
        if component.is_empty()
            || !component.bytes().all(|byte| byte.is_ascii_digit())
            || (component.len() > 1 && component.starts_with('0'))
        {
            return Err(usage(
                "Unsupported installer version; uninstall Alpha before installing 0.1.0. / 不支持此安装版本，请先卸载 Alpha 再安装 0.1.0。",
            ));
        }
        *output = component
            .parse()
            .map_err(|_| usage("Installer version is out of range / 安装版本超出范围"))?;
    }
    Ok(version)
}

fn validate_versions(previous: &str, target: &str) -> Result<()> {
    let previous = stable_version(previous)?;
    let target = stable_version(target)?;
    if previous < [0, 1, 0] || target < previous {
        return Err(usage(
            "Downgrades and pre-0.1.0 updates are not supported. / 不支持降级或从 0.1.0 之前的版本原地更新。",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn upgrade_version_matrix() {
        assert!(validate_versions("0.1.0", "0.1.0").is_ok());
        assert!(validate_versions("0.1.0", "0.1.1").is_ok());
        assert!(validate_versions("0.1.9", "0.1.10").is_ok());
        for previous in [
            "0.1.0-alpha.8",
            "0.0.9",
            "0.01.0",
            "1.2",
            "1.2.3.4",
            "4294967296.0.0",
        ] {
            assert!(validate_versions(previous, "1.2.3").is_err(), "{previous}");
        }
        assert!(validate_versions("0.1.1", "0.1.0").is_err());
    }

    #[test]
    fn installer_preserves_custom_data_paths_and_ignores_global_overrides() -> Result<()> {
        let temp = tempfile::tempdir().map_err(|_| usage("Cannot create test directory"))?;
        let root = temp.path().join("installed");
        let custom = temp.path().join("python-data");
        let config = root.join("config").join("config.toml");
        let _directory = DirectoryLease::acquire(&root.join("config"), true)?;
        let text = format!(
            "schema_version = 1\n[paths]\nruntimes_dir = {}\n",
            toml::Value::String(custom.to_string_lossy().into_owned())
        );
        WindowsStateFileSystem.write_atomic(&config, text.as_bytes())?;
        let arguments = Arguments::try_parse_from([
            "pyrudder",
            "--home",
            "Z:\\ignored",
            "--runtimes-dir",
            "Z:\\ignored-runtime",
            "__installer",
            "--root",
            root.to_str()
                .ok_or_else(|| usage("Non-Unicode test path"))?,
            "--phase",
            "check",
        ])
        .map_err(|_| usage("Invalid internal command arguments"))?;
        let app = App::load(&arguments)?;
        validate_layout(&root, &app.location)?;
        assert_eq!(
            path_key(&app.configuration.paths.runtimes_dir)?,
            path_key(&WindowsStateFileSystem.normalize_directory(&custom)?)?
        );
        assert_eq!(
            WindowsStateFileSystem
                .read_file(&config, 65_536)?
                .ok_or_else(|| usage("Missing test config"))?,
            text.as_bytes()
        );
        assert!(!root.join("config").join("registry.json").exists());
        Ok(())
    }
}
