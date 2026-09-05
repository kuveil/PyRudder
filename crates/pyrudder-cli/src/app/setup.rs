//! Explicit per-user CLI setup; no desktop application or implicit machine-wide changes.
//! 显式的当前用户 CLI 安装；不安装桌面应用或隐式修改整机设置。

use super::{App, Outcome, usage};
use pyrudder_core::{Error, ErrorKind, Result, state::StateFileSystem};
use pyrudder_platform_windows::{
    publication, router,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, sha256_file},
};
use serde_json::json;
use std::{fs, path::Path};

impl App {
    pub(super) fn setup(&self, add_to_path: bool) -> Result<Outcome> {
        let source = router::current_program_directory()?;
        let transaction = self.registry.transaction()?;
        let _target = DirectoryLease::acquire(&self.location.install_dir, true)?;
        let _shims = DirectoryLease::acquire(&self.location.shims_dir, true)?;
        self.location.publish(&self.location.install_dir)?;
        self.location.publish(&self.location.config_dir)?;
        self.location.publish(&self.location.shims_dir)?;
        for name in [
            "pyrudder.exe",
            "pyrudder-shim-console.exe",
            "pyrudder-shim-gui.exe",
        ] {
            copy_new_or_identical(&source.join(name), &self.location.install_dir.join(name))?;
        }
        let config = self.location.config_dir.join("config.toml");
        if WindowsStateFileSystem.read_file(&config, 65_536)?.is_none() {
            WindowsStateFileSystem.write_atomic(&config, self.initial_config()?.as_bytes())?;
        }
        let generation = publication::publish(&self.location, transaction)?;
        let path_changed = add_to_path && self.update_system_path(true, false)?;
        Ok(Outcome::Data(
            json!({"installed": self.location.install_dir, "paths": self.paths_json(), "generation": generation,
            "system_path_changed": path_changed, "desktop_app": false,
            "next": "Reopen your terminal. Run pyrudder register <python-path> --alias work, then pyrudder global work. Use pyrudder local work for a project or pyrudder shell work for this terminal; no initialization script is required."}),
        ))
    }
}

pub(super) fn package_home(program: &Path) -> Result<Option<std::path::PathBuf>> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Layout {
        schema_version: u32,
        layout: String,
    }
    let Some(bytes) =
        WindowsStateFileSystem.read_file(&program.join("pyrudder-layout.json"), 1024)?
    else {
        return Ok(None);
    };
    let marker: Layout =
        serde_json::from_slice(&bytes).map_err(|_| usage("Invalid package layout marker"))?;
    if marker.schema_version != 1
        || marker.layout != "in-place"
        || !program
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
    {
        return Err(usage(
            "Unsupported package layout; keep the bin directory intact",
        ));
    }
    Ok(Some(
        program
            .parent()
            .ok_or_else(|| usage("Package root is missing"))?
            .to_path_buf(),
    ))
}

fn copy_new_or_identical(source: &Path, destination: &Path) -> Result<()> {
    let digest = sha256_file(source, 128 * 1024 * 1024)?;
    if destination.exists() {
        if sha256_file(destination, 128 * 1024 * 1024)? == digest {
            return Ok(());
        }
        return Err(Error::new(
            ErrorKind::Conflict,
            "Installation destination contains a different binary; use the Setup installer to update it. / 安装目录包含不同的程序文件，请使用安装包更新。",
        ));
    }
    let mut input = fs::File::open(source).map_err(|_| usage("Cannot open setup source binary"))?;
    let parent = destination
        .parent()
        .ok_or_else(|| usage("Setup destination needs a parent"))?;
    let mut output = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| usage("Cannot stage setup destination"))?;
    std::io::copy(&mut input, &mut output).map_err(|_| usage("Cannot copy setup binary"))?;
    output
        .as_file()
        .sync_all()
        .map_err(|_| usage("Cannot flush setup binary"))?;
    let staged = output.into_temp_path();
    if sha256_file(&staged, 128 * 1024 * 1024)? != digest {
        return Err(Error::new(
            ErrorKind::Integrity,
            "Copied binary digest differs from source",
        ));
    }
    staged
        .persist_noclobber(destination)
        .map_err(|_| usage("Cannot publish setup binary without overwriting another file"))?;
    Ok(())
}
