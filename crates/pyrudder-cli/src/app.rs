//! Management orchestration over explicit configuration and atomic registry operations.
//! 基于显式配置和原子登记操作的管理编排。

mod available;
mod config;
mod download_progress;
pub(crate) mod installer;
mod installer_transaction;
mod managed;
mod path;
mod setup;
mod shell;
mod store_path;
mod system_path;

use crate::arguments::{Action, Arguments};
use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::CommandRequest,
    config::{
        ConfigEnvironment, ConfigOverrides, Configuration, PathOverrides, file::load_configuration,
    },
    resolver::{ResolveRequest, resolve},
    runtime::{RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord},
    selector::{RuntimeSelection, VersionSelector},
    state::{StateFileSystem, read_selection_file, write_selection_file},
};
use pyrudder_platform_windows::{
    commands::discover_commands,
    probe, publication,
    registry::{Location, Registry, Snapshot},
    router, session,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, new_identity, path_key},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
};

pub(crate) enum Outcome {
    Data(Value),
    Text(String),
    Child(i32),
}

pub(crate) struct App {
    configuration: Configuration,
    location: Location,
    registry: Registry,
    progress: bool,
    interactive: bool,
    _installation_access: Option<FileLease>,
}

fn configuration_sources(arguments: &Arguments) -> (ConfigEnvironment, ConfigOverrides) {
    let mut environment = ConfigEnvironment::capture();
    if let Some(home) = &arguments.home {
        environment.home = Some(home.clone());
    }
    let mut overrides = ConfigOverrides {
        paths: PathOverrides {
            install_dir: arguments.install_dir.clone(),
            runtimes_dir: arguments.runtimes_dir.clone(),
            downloads_dir: arguments.downloads_dir.clone(),
            cache_dir: arguments.cache_dir.clone(),
            temp_dir: arguments.temp_dir.clone(),
            shims_dir: arguments.shims_dir.clone(),
            config_dir: arguments.config_dir.clone(),
        },
        system_fallback: None,
    };
    if let Some(Action::Installer { root, .. }) = &arguments.command {
        // Installer identity comes from its explicit root, never inherited environment.
        // 安装器身份只由显式根目录决定，绝不读取继承的环境覆盖。
        environment = ConfigEnvironment {
            home: Some(root.clone()),
            ..ConfigEnvironment::default()
        };
        overrides = ConfigOverrides {
            paths: PathOverrides {
                config_dir: Some(root.join("config")),
                ..PathOverrides::default()
            },
            system_fallback: None,
        };
    }
    (environment, overrides)
}

impl App {
    pub(crate) fn load(arguments: &Arguments) -> Result<Self> {
        let (mut environment, mut overrides) = configuration_sources(arguments);
        if overrides.paths.config_dir.is_none()
            && environment.paths.config_dir.is_none()
            && environment.home.is_none()
        {
            let program = router::current_program_directory()?;
            let package_home = setup::package_home(&program)?;
            if WindowsStateFileSystem
                .read_file(&program.join("pyrudder-location.json"), 65_536)?
                .is_some()
            {
                let installed = Location::read(&program)?;
                if package_home.is_some()
                    && path_key(&installed.install_dir)? != path_key(&program)?
                {
                    return Err(usage(
                        "Initialized package was moved; restore its original directory. Automatic migration is not supported.",
                    ));
                }
                overrides.paths.config_dir = Some(installed.config_dir);
                // Unset settings inherit this package's root, including after setup published its locator.
                // setup 发布位置契约后，取消的路径配置仍应回退到当前安装根目录。
                environment.home = package_home;
            } else if let Some(home) = package_home {
                environment.home = Some(home);
            }
        }
        let configuration = load_configuration(&WindowsStateFileSystem, &environment, &overrides)?;
        let paths = &configuration.paths;
        let keys = [
            &paths.config_dir,
            &paths.install_dir,
            &paths.shims_dir,
            &paths.runtimes_dir,
            &paths.downloads_dir,
            &paths.cache_dir,
            &paths.temp_dir,
        ]
        .into_iter()
        .map(|path| path_key(path))
        .collect::<Result<Vec<_>>>()?;
        for (index, left) in keys.iter().enumerate() {
            if keys
                .iter()
                .skip(index + 1)
                .any(|right| left.starts_with(right) || right.starts_with(left))
            {
                return Err(usage(
                    "Program, config, shims, runtimes, downloads, cache and temp directories must be distinct and non-overlapping",
                ));
            }
        }
        let location = Location {
            schema_version: 1,
            config_dir: paths.config_dir.clone(),
            shims_dir: paths.shims_dir.clone(),
            install_dir: paths.install_dir.clone(),
        };
        location.validate()?;
        let installation_access = if matches!(arguments.command, Some(Action::Installer { .. })) {
            None
        } else {
            pyrudder_platform_windows::installation::shared_access(&location)?
        };
        let registry = Registry::new(&paths.config_dir)?;
        if WindowsStateFileSystem
            .read_file(&paths.config_dir.join("pyrudder-location.json"), 65_536)?
            .is_some()
            && Location::read(&paths.config_dir)? != location
        {
            return Err(usage(
                "Program/shim location changed; do not abandon existing state. Use a separate --home for a new installation.",
            ));
        }
        Ok(Self {
            configuration,
            location,
            registry,
            progress: !arguments.quiet && !arguments.json,
            interactive: !arguments.quiet
                && !arguments.json
                && io::stdin().is_terminal()
                && io::stdout().is_terminal()
                && io::stderr().is_terminal(),
            _installation_access: installation_access,
        })
    }

    pub(crate) fn execute(&self, action: &Action) -> Result<Outcome> {
        let data = match action {
            Action::Installer { .. } => {
                return Err(usage("Installer action requires its dedicated entry point"));
            }
            Action::Setup { add_to_path } => return self.setup(*add_to_path),
            Action::Path { action } => self.path(action)?,
            Action::SystemPath { owner_sid, remove } => {
                if *remove {
                    let store = self.repair_store_path(true, Some(owner_sid))?;
                    let system = self.update_system_path(false, true)?;
                    json!({"system_path_changed": system, "store_path": store, "restart_terminal": true})
                } else {
                    let system = self.update_system_path(true, true)?;
                    let store = self.repair_store_path(false, Some(owner_sid))?;
                    json!({"system_path_changed": system, "store_path": store, "restart_terminal": true})
                }
            }
            Action::Register {
                path,
                alias,
                scan,
                allow_venv,
            } => self.register(path, alias.as_deref(), *scan, *allow_venv)?,
            Action::Unregister {
                version,
                force,
                yes,
            } => self.unregister(version, *force, *yes)?,
            Action::List => {
                json!({"runtimes": self.registry.load()?.runtimes.iter().map(runtime_json).collect::<Vec<_>>() })
            }
            Action::Available { offline, list } => return self.available(*offline, *list),
            Action::Download { version, offline } => self.download(version, *offline)?,
            Action::Install {
                version,
                alias,
                offline,
            } => self.install(version, alias.as_deref(), *offline)?,
            Action::Uninstall {
                version,
                force,
                yes,
            } => self.uninstall(version, *force, *yes)?,
            Action::Cache { action } => self.cache(action)?,
            Action::Global { version, unset } => {
                self.selection(version.as_deref(), false, *unset)?
            }
            Action::Local { version, unset } => self.selection(version.as_deref(), true, *unset)?,
            Action::Current { explain: _ } => self.current()?,
            Action::Which {
                command,
                all,
                explain: _,
            } => self.which(command, *all)?,
            Action::Info { version } => runtime_json(find_any(&self.registry.load()?, version)?),
            Action::Commands { version } => self.commands(version.as_deref())?,
            Action::Exec { version, arguments } => {
                let (command, forwarded) = arguments
                    .split_first()
                    .ok_or_else(|| usage("Expected exec -- <command> [args...]"))?;
                let command = command
                    .to_str()
                    .ok_or_else(|| usage("Command name must be Unicode"))?;
                let exit = router::execute(
                    &self.location,
                    &router::command_request(command)?,
                    version.as_deref(),
                    forwarded,
                )?;
                return Ok(Outcome::Child(exit.for_process_exit()));
            }
            Action::Rehash {
                version,
                check,
                dry_run,
            } => self.rehash(version.as_deref(), *check, *dry_run)?,
            Action::Doctor => self.doctor()?,
            Action::Recover {
                discard_staging,
                yes,
            } => self.recover(discard_staging.as_deref(), *yes)?,
            Action::Completions { shell } => return Ok(Outcome::Text(Self::completions(*shell)?)),
            Action::Config { action } => self.config(action)?,
            Action::Shell { version, unset } => self.shell_selection(version.as_deref(), *unset)?,
            Action::Dispatch {
                filename,
                arguments,
            } => {
                return Ok(Outcome::Child(
                    router::run_wrapper(filename, arguments)?.for_process_exit(),
                ));
            }
        };
        Ok(Outcome::Data(data))
    }

    fn select<'a>(
        &self,
        snapshot: &'a Snapshot,
        explicit: Option<&str>,
    ) -> Result<&'a RuntimeRecord> {
        let cwd = cwd()?;
        let shell = if explicit.is_some() {
            None
        } else {
            session::selection_value(&self.location.config_dir)?
        };
        let global = self.location.config_dir.join("global-version");
        let request = ResolveRequest {
            explicit,
            shell_version: shell.as_deref(),
            cwd: &cwd,
            workspace_boundary: None,
            global_file: &global,
            system_fallback: self.configuration.system_fallback,
        };
        match resolve(&WindowsStateFileSystem, &request, &snapshot.runtimes)
            .result?
            .selection
        {
            RuntimeSelection::Registered(record) => Ok(record),
            RuntimeSelection::SystemRequested => Err(usage(
                "This operation requires a registered runtime, not system",
            )),
        }
    }

    fn register(
        &self,
        path: &Path,
        alias: Option<&str>,
        scan: bool,
        allow_venv: bool,
    ) -> Result<Value> {
        if scan && alias.is_some() {
            return Err(usage("--alias cannot be combined with --scan"));
        }
        let candidates = if scan {
            scan_candidates(path)?
        } else {
            vec![probe::executable_path(path)?]
        };
        let mut records = Vec::new();
        let mut skipped = Vec::new();
        for executable in candidates {
            match probe::inspect(&executable, allow_venv) {
                Ok(identity) => {
                    let id = RuntimeId::new(identity.version.parse()?)?
                        .with_installation(new_identity()?);
                    let root = executable
                        .parent()
                        .ok_or_else(|| usage("Interpreter has no root"))?
                        .to_path_buf();
                    let mut record = RuntimeRecord::new(
                        id,
                        root,
                        identity.scripts,
                        RuntimeOrigin::External {
                            registered_executable: executable,
                        },
                    )?;
                    if let Some(alias) = alias {
                        record.add_alias(alias.parse()?);
                    }
                    record.set_health(RuntimeHealth::Ready);
                    records.push(record);
                }
                Err(error) if scan => {
                    skipped.push(json!({"path": executable, "error": error.message()}));
                }
                Err(error) => return Err(error),
            }
        }
        if records.is_empty() {
            return Err(usage(
                "No supported Python interpreters found in the explicitly supplied directory",
            ));
        }
        let mut transaction = self.registry.transaction()?;
        let mut added = Vec::new();
        for record in records {
            if transaction
                .snapshot
                .runtimes
                .iter()
                .any(|existing| path_key(existing.root()).ok() == path_key(record.root()).ok())
            {
                if scan {
                    skipped.push(json!({"path": record.root(), "error": "already registered"}));
                    continue;
                }
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "This Python root is already registered",
                ));
            }
            let _lease = FileLease::acquire(&self.registry.lease_path(record.id()), true, true)?;
            let manifest = discover_commands(&record)?;
            added.push(runtime_json(&record));
            transaction.snapshot.runtimes.push(record);
            transaction.snapshot.manifests.push(manifest);
        }
        let generation = publication::publish(&self.location, transaction)?;
        Ok(
            json!({"registered": added, "skipped": skipped, "generation": generation, "hint": "Select one with pyrudder global <version> or pyrudder local <version>."}),
        )
    }

    fn unregister(&self, version: &str, force: bool, yes: bool) -> Result<Value> {
        if !yes {
            return Err(usage(
                "Use --yes to remove a registration; external Python files are never deleted",
            ));
        }
        let mut transaction = self.registry.transaction()?;
        let record = find_any(&transaction.snapshot, version)?.clone();
        if !matches!(record.origin(), RuntimeOrigin::External { .. }) {
            return Err(usage("Managed runtimes must use uninstall, not unregister"));
        }
        if !force {
            self.check_active(record.id())?;
        }
        let _lease = FileLease::acquire(&self.registry.lease_path(record.id()), true, false)?;
        transaction
            .snapshot
            .runtimes
            .retain(|candidate| candidate.id() != record.id());
        transaction
            .snapshot
            .manifests
            .retain(|manifest| manifest.runtime_id() != record.id());
        publication::publish(&self.location, transaction)?;
        Ok(
            json!({"unregistered": record.id().to_string(), "preserved_external_root": record.root()}),
        )
    }

    fn check_active(&self, id: &RuntimeId) -> Result<()> {
        let snapshot = self.registry.load()?;
        let global = read_selection_file(
            &WindowsStateFileSystem,
            &self.location.config_dir.join("global-version"),
        )?;
        let shell = session::selection_value(&self.location.config_dir)?.and_then(|value| {
            value
                .to_str()
                .and_then(|text| text.parse::<VersionSelector>().ok())
        });
        if global.iter().chain(shell.iter()).any(|selection| {
            find_any(&snapshot, &selection.to_string()).is_ok_and(|record| record.id() == id)
        }) {
            return Err(Error::new(
                ErrorKind::Busy,
                "Runtime is selected globally or in this shell; switch first or use --force",
            ));
        }
        Ok(())
    }

    fn selection(&self, version: Option<&str>, local: bool, unset: bool) -> Result<Value> {
        let path = if local {
            cwd()?.join(".python-version")
        } else {
            self.location.config_dir.join("global-version")
        };
        if unset && version.is_some() {
            return Err(usage("A version and --unset cannot be combined"));
        }
        if unset {
            let _anchor = DirectoryLease::acquire(
                path.parent()
                    .ok_or_else(|| usage("Selection requires a parent"))?,
                false,
            )?;
            if read_selection_file(&WindowsStateFileSystem, &path)?.is_some() {
                fs::remove_file(&path)
                    .map_err(|_| usage("Cannot remove the validated local selection file"))?;
            }
            return Ok(json!({"removed": path}));
        }
        if let Some(version) = version {
            let transaction = self.registry.transaction()?;
            let _anchor = DirectoryLease::acquire(
                path.parent()
                    .ok_or_else(|| usage("Selection requires a parent"))?,
                false,
            )?;
            let pinned = write_selection_file(
                &WindowsStateFileSystem,
                &path,
                &version.parse()?,
                &transaction.snapshot.runtimes,
                false,
            )?;
            return Ok(json!({"selection": pinned.to_string(), "file": path}));
        }
        Ok(
            json!({"selection": read_selection_file(&WindowsStateFileSystem, &path)?.map(|selection| selection.to_string()), "file": path}),
        )
    }

    fn current(&self) -> Result<Value> {
        let snapshot = self.registry.load()?;
        let cwd = cwd()?;
        let shell = session::selection_value(&self.location.config_dir)?;
        let global = self.location.config_dir.join("global-version");
        let report = resolve(
            &WindowsStateFileSystem,
            &ResolveRequest {
                explicit: None,
                shell_version: shell.as_deref(),
                cwd: &cwd,
                workspace_boundary: None,
                global_file: &global,
                system_fallback: false,
            },
            &snapshot.runtimes,
        );
        let selected = report.result?;
        let RuntimeSelection::Registered(record) = selected.selection else {
            return Err(usage("No registered runtime selected"));
        };
        Ok(
            json!({"runtime": runtime_json(record), "source": format!("{:?}", selected.source),
            "trace": report.steps.iter().map(|step| json!({"source": format!("{:?}", step.source), "outcome": format!("{:?}", step.outcome)})).collect::<Vec<_>>(),
            "virtual_env": std::env::var_os("VIRTUAL_ENV"), "path_python": first_path_command("python.exe")}),
        )
    }

    fn which(&self, name: &str, all: bool) -> Result<Value> {
        let snapshot = self.registry.load()?;
        let request = router::command_request(name)?;
        let record = self.select(&snapshot, None)?;
        let target =
            pyrudder_core::commands::lookup_command(&snapshot.manifests, record.id(), &request)?;
        let providers = if all {
            snapshot.manifests.iter().filter_map(|manifest| manifest.lookup(&request).ok().map(|target| json!({"runtime": manifest.runtime_id().to_string(), "path": target.path}))).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(
            json!({"runtime": record.id().to_string(), "target": target.path, "kind": format!("{:?}", target.status), "providers": providers,
            "virtual_env": std::env::var_os("VIRTUAL_ENV"), "path_match": first_path_command(name)}),
        )
    }

    fn commands(&self, version: Option<&str>) -> Result<Value> {
        let snapshot = self.registry.load()?;
        let record = self.select(&snapshot, version)?;
        let manifest = snapshot
            .manifests
            .iter()
            .find(|manifest| manifest.runtime_id() == record.id())
            .ok_or_else(|| usage("Missing manifest; run rehash"))?;
        Ok(
            json!({"runtime": record.id().to_string(), "commands": manifest.commands().iter().map(|(request, entry)| {
            let (name, exact) = match request { CommandRequest::Bare(key) => (key.as_str(), false), CommandRequest::Exact(key) => (key.as_str(), true) };
            json!({"name": name, "exact": exact, "path": entry.winner.path, "status": format!("{:?}", entry.winner.status), "shadowed": entry.shadowed.iter().map(|target| &target.path).collect::<Vec<_>>()})
        }).collect::<Vec<_>>() }),
        )
    }

    fn rehash(&self, version: Option<&str>, check: bool, dry_run: bool) -> Result<Value> {
        if check || dry_run {
            let mut snapshot = self.registry.load()?;
            let before = snapshot.shims.clone();
            self.rescan(&mut snapshot, version, false)?;
            let expected = publication::desired_index(&self.location, &snapshot)?;
            let drift = before != expected || snapshot.manifests != self.registry.load()?.manifests;
            return Ok(json!({"changed": drift, "expected_shims": expected, "writes": false}));
        }
        let mut transaction = self.registry.transaction()?;
        self.rescan(&mut transaction.snapshot, version, true)?;
        let generation = publication::publish(&self.location, transaction)?;
        Ok(json!({"generation": generation, "message": "Command index refreshed"}))
    }

    fn rescan(
        &self,
        snapshot: &mut Snapshot,
        version: Option<&str>,
        probe_identity: bool,
    ) -> Result<()> {
        let selected = version
            .map(|version| find_any(snapshot, version).map(|record| record.id().clone()))
            .transpose()?;
        for record in &snapshot.runtimes {
            if !matches!(record.health(), RuntimeHealth::Ready) {
                continue;
            }
            if selected.as_ref().is_some_and(|id| id != record.id()) {
                continue;
            }
            let _lease = FileLease::acquire(&self.registry.lease_path(record.id()), false, false)?;
            if probe_identity {
                let executable = match record.origin() {
                    RuntimeOrigin::External {
                        registered_executable,
                    } => registered_executable.clone(),
                    RuntimeOrigin::Managed { .. } => record.root().join("python.exe"),
                };
                let identity = probe::inspect(&executable, true)?;
                if identity.version != record.id().version().to_string() {
                    return Err(Error::new(
                        ErrorKind::BrokenRuntime,
                        "Interpreter version changed in place; remove its old registration and register it again",
                    ));
                }
            }
            let manifest = discover_commands(record)?;
            snapshot
                .manifests
                .retain(|previous| previous.runtime_id() != record.id());
            snapshot.manifests.push(manifest);
        }
        Ok(())
    }

    fn doctor(&self) -> Result<Value> {
        let snapshot = self.registry.load()?;
        let mut warnings = Vec::new();
        for record in &snapshot.runtimes {
            if !matches!(record.health(), RuntimeHealth::Ready) {
                warnings.push(format!(
                    "{}: {:?}; inspect recover and resume install/uninstall",
                    record.id(),
                    record.health()
                ));
                continue;
            }
            match discover_commands(record) {
                Ok(actual)
                    if snapshot
                        .manifests
                        .iter()
                        .find(|manifest| manifest.runtime_id() == record.id())
                        .is_none_or(|stored| stored.directories() != actual.directories()) =>
                {
                    warnings.push(format!(
                        "{}: command inventory drift; run rehash",
                        record.id()
                    ));
                }
                Err(error) => warnings.push(format!("{}: {error}", record.id())),
                _ => {}
            }
        }
        if std::env::var_os("PYTHONHOME").is_some() || std::env::var_os("PYTHONPATH").is_some() {
            warnings.push("PYTHONHOME/PYTHONPATH is set and is inherited by Python".into());
        }
        if std::env::var_os("VIRTUAL_ENV").is_some() {
            warnings.push(
                "An active virtual environment should take PATH priority over PyRudder shims"
                    .into(),
            );
        }
        let path_python = first_path_command("python.exe");
        if let Some(path) = &path_python {
            if !path_key(path)?.starts_with(path_key(&self.location.shims_dir)?) {
                warnings.push("PATH resolves python outside PyRudder. Outside an active virtual environment, rerun the installer and reopen the terminal; use pyrudder exec -- python <args> for explicit routing. / PATH 当前命中的 python 不属于 PyRudder。若未激活虚拟环境，请重新运行安装器并重开终端；也可用 pyrudder exec -- python <参数> 明确路由。".into());
            }
        }
        if !snapshot.pending_cleanup.is_empty() {
            warnings.push(
                "Owned stale shims await cleanup; run rehash when files are no longer occupied"
                    .into(),
            );
        }
        Ok(
            json!({"healthy": warnings.is_empty(), "warnings": warnings, "runtime_count": snapshot.runtimes.len(), "generation": snapshot.generation,
            "location": self.location, "path_python": path_python, "pending_cleanup": snapshot.pending_cleanup, "note": "Read-only filesystem diagnostics; no interpreter executed. Shell aliases/functions can override PATH and must also be checked in the shell."}),
        )
    }
}

pub(super) fn usage(message: &str) -> Error {
    Error::new(ErrorKind::Usage, message)
}
fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|_| usage("Cannot read current directory"))
}

fn runtime_json(record: &RuntimeRecord) -> Value {
    json!({"id": record.id().to_string(), "version": record.id().version().to_string(), "root": record.root(), "scripts": record.command_dirs(),
        "origin": if matches!(record.origin(), RuntimeOrigin::External { .. }) { "external" } else { "managed" },
        "health": format!("{:?}", record.health()), "aliases": record.aliases().iter().map(ToString::to_string).collect::<Vec<_>>()})
}

fn find_any<'a>(snapshot: &'a Snapshot, version: &str) -> Result<&'a RuntimeRecord> {
    if let Ok(id) = version.parse::<RuntimeId>() {
        return snapshot
            .runtimes
            .iter()
            .find(|record| record.id() == &id)
            .ok_or_else(|| Error::new(ErrorKind::NotInstalled, "Runtime is not registered"));
    }
    let mut records = snapshot.runtimes.clone();
    for record in &mut records {
        record.set_health(RuntimeHealth::Ready);
    }
    let selector: VersionSelector = version.parse()?;
    let RuntimeSelection::Registered(selected) = selector.select(&records)? else {
        return Err(usage("Expected a registered runtime"));
    };
    snapshot
        .runtimes
        .iter()
        .find(|record| record.id() == selected.id())
        .ok_or_else(|| usage("Missing selected registration"))
}

fn scan_candidates(root: &Path) -> Result<Vec<PathBuf>> {
    let root = WindowsStateFileSystem.canonical_directory(root)?;
    let mut paths = Vec::new();
    if root.join("python.exe").is_file() {
        paths.push(root.join("python.exe"));
    }
    for (index, entry) in fs::read_dir(&root)
        .map_err(|_| usage("Cannot enumerate registration directory"))?
        .enumerate()
    {
        if index >= 1024 {
            return Err(usage("Registration scan exceeds 1024 direct entries"));
        }
        let entry = entry.map_err(|_| usage("Cannot read registration candidate"))?;
        if entry
            .file_type()
            .map_err(|_| usage("Cannot inspect registration candidate"))?
            .is_dir()
        {
            let path = entry.path().join("python.exe");
            if path.is_file() {
                paths.push(probe::executable_path(&path)?);
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn first_path_command(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = if Path::new(name).extension().is_some() {
        name.to_owned()
    } else {
        format!("{name}.exe")
    };
    std::env::split_paths(&path)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(&name))
        .find(|path| path.is_file())
}
