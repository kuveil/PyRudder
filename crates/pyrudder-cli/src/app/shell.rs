//! Native terminal selection and optional completion output; no initialization script is needed.
//! 原生终端选择与可选补全输出，不需要初始化脚本。

use super::{App, usage};
use crate::arguments::Shell;
use pyrudder_core::{
    Result,
    state::{MAX_SELECTION_FILE_BYTES, StateFileSystem, write_selection_file},
};
use pyrudder_platform_windows::{session, state::WindowsStateFileSystem, storage::DirectoryLease};
use serde_json::{Value, json};

impl App {
    pub(super) fn shell_selection(&self, version: Option<&str>, unset: bool) -> Result<Value> {
        if unset && version.is_some() {
            return Err(usage("A version and --unset cannot be combined"));
        }
        if version.is_none() && !unset {
            return self.current();
        }
        if std::env::var_os("PYRUDDER_VERSION").is_some() {
            return Err(usage(
                "This process inherits an explicit Python selection. Open a fresh terminal for independent pyrudder shell selection, or use pyrudder exec --version <version> -- <command>.",
            ));
        }
        let path = session::selection_path(&self.location.config_dir)?
            .ok_or_else(|| usage("Cannot identify this terminal. Run from PowerShell or CMD, or use pyrudder local <version> / pyrudder exec --version <version> -- <command>."))?;
        let parent = path
            .parent()
            .ok_or_else(|| usage("Terminal selection requires a parent directory"))?;
        // Serialize selection publication with registration/removal; pin before writing.
        // 与登记和移除操作串行发布选择，写入前固定安装标识。
        let transaction = self.registry.transaction()?;
        let _anchor = DirectoryLease::acquire(parent, true)?;
        if unset {
            WindowsStateFileSystem.read_file(&path, MAX_SELECTION_FILE_BYTES)?;
            // A tombstone prevents a nested shell inheriting its parent's selection again.
            // 清除标记防止嵌套终端再次继承外层选择。
            WindowsStateFileSystem.write_atomic(&path, session::CLEARED_SELECTION)?;
            return Ok(
                json!({"scope": "shell", "cleared": true, "initialization_required": false}),
            );
        }
        let selector = version
            .ok_or_else(|| usage("Expected a Python version"))?
            .parse()?;
        let pinned = write_selection_file(
            &WindowsStateFileSystem,
            &path,
            &selector,
            &transaction.snapshot.runtimes,
            false,
        )?;
        Ok(
            json!({"scope": "shell", "selection": pinned.to_string(), "initialization_required": false,
            "message": "Python selected for this terminal and its descendants. Independent terminals and project/global choices were not changed."}),
        )
    }

    pub(super) fn completions(shell: Shell) -> Result<String> {
        use clap::CommandFactory;
        if matches!(shell, Shell::Cmd) {
            return Err(usage(
                "CMD has no native programmable completion; use pyrudder --help",
            ));
        }
        let mut bytes = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::PowerShell,
            &mut crate::arguments::Arguments::command(),
            "pyrudder",
            &mut bytes,
        );
        String::from_utf8(bytes).map_err(|_| usage("Cannot encode completion script"))
    }
}
