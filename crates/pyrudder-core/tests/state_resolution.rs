//! Deterministic configuration, selection-file, and scope-resolution tests.
//! 确定性的配置、选择文件与作用域解析测试。

use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

use pyrudder_core::{
    Error, ErrorKind, Result,
    resolver::{ResolveRequest, TraceOutcome, VersionSource, resolve},
    runtime::{InstallationId, RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord},
    selector::RuntimeSelection,
    state::{StateFileSystem, parse_selection_file, read_selection_file, write_selection_file},
};

#[derive(Default)]
struct MemoryFs {
    files: RefCell<BTreeMap<PathBuf, Vec<u8>>>,
    reads: RefCell<Vec<PathBuf>>,
    fail_reads: bool,
    fail_writes: bool,
}

impl MemoryFs {
    fn insert(&self, path: impl Into<PathBuf>, bytes: impl AsRef<[u8]>) {
        self.files
            .borrow_mut()
            .insert(path.into(), bytes.as_ref().to_vec());
    }
}

impl StateFileSystem for MemoryFs {
    fn read_file(&self, path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        self.reads.borrow_mut().push(path.to_path_buf());
        if self.fail_reads {
            return Err(Error::new(ErrorKind::Permission, "Injected read failure"));
        }
        let bytes = self.files.borrow().get(path).cloned();
        if bytes.as_ref().is_some_and(|value| value.len() > max_bytes) {
            return Err(Error::new(ErrorKind::Usage, "Oversized file"));
        }
        Ok(bytes)
    }
    fn normalize_directory(&self, path: &Path) -> Result<PathBuf> {
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err(Error::new(ErrorKind::Usage, "Invalid path"));
        }
        Ok(path.to_path_buf())
    }
    fn canonical_directory(&self, path: &Path) -> Result<PathBuf> {
        self.normalize_directory(path)
    }
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<()> {
        if self.fail_writes {
            return Err(Error::new(ErrorKind::Busy, "Injected publication failure"));
        }
        self.insert(path, contents);
        Ok(())
    }
}

fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\model-tests")
    } else {
        PathBuf::from("/model-tests")
    }
}

fn runtime(version: &str, installation: u128) -> Result<RuntimeRecord> {
    let id =
        RuntimeId::new(version.parse()?)?.with_installation(InstallationId::new(installation)?);
    let root = root().join(id.to_string());
    let mut runtime = RuntimeRecord::new(
        id,
        root.clone(),
        vec![root.join("Scripts")],
        RuntimeOrigin::External {
            registered_executable: root.join("python.exe"),
        },
    )?;
    runtime.set_health(RuntimeHealth::Ready);
    Ok(runtime)
}

fn request<'a>(cwd: &'a Path, global: &'a Path) -> ResolveRequest<'a> {
    ResolveRequest {
        explicit: None,
        shell_version: None,
        cwd,
        workspace_boundary: Some(cwd),
        global_file: global,
        system_fallback: false,
    }
}

fn assert_kind<T>(result: Result<T>, kind: ErrorKind) {
    assert!(matches!(result, Err(error) if error.kind() == kind));
}

#[test]
fn selection_files_accept_only_one_optional_bom_and_line_terminator() -> Result<()> {
    for text in [
        "3.13",
        "3.13\n",
        "3.13\r\n",
        "\u{feff}3.13",
        "\u{feff}3.13\r\n",
    ] {
        assert_eq!(parse_selection_file(text.as_bytes())?.to_string(), "3.13");
    }
    for bytes in [
        b"".as_slice(),
        b"\xff",
        b"\n",
        b"3.13\r",
        b"3.13\n\n",
        b"3.13\r\n3.12",
        b"3.13 ",
        b" 3.13",
        b"3.13\0",
        b"3.13 # comment",
        b"$(command)",
        b"https://example.org",
        b"./python.exe",
    ] {
        assert_kind(parse_selection_file(bytes), ErrorKind::Usage);
    }
    assert_kind(
        parse_selection_file("\u{feff}\u{feff}3.13".as_bytes()),
        ErrorKind::Usage,
    );
    assert_kind(parse_selection_file(&[b'a'; 134]), ErrorKind::Usage);
    Ok(())
}

#[test]
fn bad_file_hints_escape_control_sequences() {
    let fs = MemoryFs::default();
    let path = root().join("secret\u{1b}[31m/.python-version");
    fs.insert(&path, b"invalid input");
    assert!(
        matches!(read_selection_file(&fs, &path), Err(error) if !error.to_string().contains('\u{1b}'))
    );
}

#[test]
fn all_scope_presence_combinations_follow_priority() -> Result<()> {
    let cwd = root().join("project");
    let global = root().join("global-version");
    let records = [
        runtime("3.10.1", 1)?,
        runtime("3.11.1", 2)?,
        runtime("3.12.1", 3)?,
        runtime("3.13.1", 4)?,
    ];
    for mask in 0_u8..16 {
        let fs = MemoryFs::default();
        let mut input = request(&cwd, &global);
        input.explicit = (mask & 1 != 0).then_some("3.10");
        input.shell_version = (mask & 2 != 0).then_some(OsStr::new("3.11"));
        if mask & 4 != 0 {
            fs.insert(cwd.join(".python-version"), b"3.12\r\n");
        }
        if mask & 8 != 0 {
            fs.insert(&global, b"3.13\n");
        }
        let report = resolve(&fs, &input, &records);
        if mask == 0 {
            assert_kind(report.result, ErrorKind::NotInstalled);
        } else {
            let index = usize::try_from(mask.trailing_zeros())
                .map_err(|_| Error::new(ErrorKind::Internal, "Invalid test index"))?;
            assert_eq!(
                report.result?.selection,
                RuntimeSelection::Registered(&records[index])
            );
            assert_eq!(
                report.steps.last().map(|step| step.outcome),
                Some(TraceOutcome::Selected)
            );
        }
    }
    Ok(())
}

#[test]
fn explicit_selection_never_reads_broken_lower_priority_sources() -> Result<()> {
    let fs = MemoryFs {
        fail_reads: true,
        ..MemoryFs::default()
    };
    let cwd = PathBuf::from("invalid-relative-cwd");
    let global = PathBuf::from("invalid-relative-global");
    let records = [runtime("3.13.7", 1)?];
    let mut input = request(&cwd, &global);
    input.explicit = Some("3.13");
    input.shell_version = Some(OsStr::new("bad shell selector"));
    let report = resolve(&fs, &input, &records);
    assert_eq!(report.result?.source, VersionSource::Explicit);
    assert_eq!(report.steps.len(), 1);
    assert!(fs.reads.borrow().is_empty());
    Ok(())
}

#[test]
fn invalid_or_missing_high_priority_selection_never_falls_back() -> Result<()> {
    let fs = MemoryFs::default();
    let cwd = root();
    let global = cwd.join("global-version");
    fs.insert(cwd.join(".python-version"), b"3.13");
    fs.insert(&global, b"3.13");
    let records = [runtime("3.13.7", 1)?];
    for value in ["", "bad selector", "3.12"] {
        for explicit in [false, true] {
            let mut input = request(&cwd, &global);
            input.system_fallback = true;
            if explicit {
                input.explicit = Some(value);
            } else {
                input.shell_version = Some(OsStr::new(value));
            }
            let report = resolve(&fs, &input, &records);
            assert_kind(
                report.result,
                if value == "3.12" {
                    ErrorKind::NotInstalled
                } else {
                    ErrorKind::Usage
                },
            );
            assert_eq!(
                report.steps.last().map(|step| step.outcome),
                Some(TraceOutcome::Rejected)
            );
            assert!(fs.reads.borrow().is_empty());
        }
    }
    Ok(())
}

#[test]
fn nearest_project_wins_and_explicit_boundary_is_inclusive() -> Result<()> {
    let fs = MemoryFs::default();
    let boundary = root().join("workspace");
    let cwd = boundary.join("project/src");
    let project = boundary.join("project");
    let global = root().join("global-version");
    let records = [runtime("3.12.1", 1)?, runtime("3.13.7", 2)?];
    fs.insert(boundary.join(".python-version"), b"3.12");
    fs.insert(project.join(".python-version"), b"3.13");
    let mut input = request(&cwd, &global);
    input.workspace_boundary = Some(&boundary);
    let report = resolve(&fs, &input, &records);
    assert_eq!(
        report.result?.source,
        VersionSource::LocalFile(project.join(".python-version"))
    );
    assert_eq!(
        &*fs.reads.borrow(),
        &[cwd.join(".python-version"), project.join(".python-version")]
    );
    fs.files
        .borrow_mut()
        .remove(&project.join(".python-version"));
    assert_eq!(
        resolve(&fs, &input, &records).result?.source,
        VersionSource::LocalFile(boundary.join(".python-version"))
    );
    Ok(())
}

#[test]
fn boundary_prevents_parent_leakage_and_rejects_sibling_prefixes() -> Result<()> {
    let fs = MemoryFs::default();
    let boundary = root().join("work");
    let cwd = boundary.join("src");
    let global = root().join("global-version");
    fs.insert(root().join(".python-version"), b"3.12");
    fs.insert(&global, b"3.13");
    let records = [runtime("3.13.7", 1)?];
    let mut input = request(&cwd, &global);
    input.workspace_boundary = Some(&boundary);
    assert_eq!(
        resolve(&fs, &input, &records).result?.source,
        VersionSource::GlobalFile(global.clone())
    );
    assert!(!fs.reads.borrow().contains(&root().join(".python-version")));
    let outside = root().join("workspace");
    input.cwd = &outside;
    assert_kind(resolve(&fs, &input, &records).result, ErrorKind::Usage);
    Ok(())
}

#[test]
fn absent_boundary_searches_to_root_and_excessive_depth_is_bounded() -> Result<()> {
    let fs = MemoryFs::default();
    let cwd = root();
    let global = cwd.join("global-version");
    let filesystem_root = cwd
        .ancestors()
        .last()
        .ok_or_else(|| Error::new(ErrorKind::Internal, "Missing root"))?;
    fs.insert(filesystem_root.join(".python-version"), b"3.13");
    let records = [runtime("3.13.7", 1)?];
    let mut input = request(&cwd, &global);
    input.workspace_boundary = None;
    assert_eq!(
        resolve(&fs, &input, &records).result?.source,
        VersionSource::LocalFile(filesystem_root.join(".python-version"))
    );
    let deep = (0..256).fold(root(), |path, _| path.join("d"));
    input.cwd = &deep;
    assert_kind(resolve(&fs, &input, &records).result, ErrorKind::Usage);
    fs.insert(deep.join(".python-version"), b"3.13");
    assert_eq!(
        resolve(&fs, &input, &records).result?.source,
        VersionSource::LocalFile(deep.join(".python-version"))
    );
    fs.files.borrow_mut().remove(&deep.join(".python-version"));
    input.workspace_boundary = Some(&deep);
    assert_kind(
        resolve(&fs, &input, &records).result,
        ErrorKind::NotInstalled,
    );
    Ok(())
}

#[test]
fn malformed_nearest_file_blocks_parent_global_and_system() -> Result<()> {
    let fs = MemoryFs::default();
    let cwd = root().join("project");
    let global = root().join("global-version");
    fs.insert(&global, b"3.13");
    fs.insert(root().join(".python-version"), b"3.13");
    let records = [runtime("3.13.7", 1)?];
    let mut input = request(&cwd, &global);
    input.workspace_boundary = None;
    input.system_fallback = true;
    for bytes in [b"".as_slice(), b"\xff", b"3.13\n3.12", &[b'a'; 134]] {
        fs.insert(cwd.join(".python-version"), bytes);
        fs.reads.borrow_mut().clear();
        let report = resolve(&fs, &input, &records);
        assert_kind(report.result, ErrorKind::Usage);
        assert_eq!(fs.reads.borrow().len(), 1);
        assert_eq!(
            report.steps.last().map(|step| &step.source),
            Some(&VersionSource::LocalFile(cwd.join(".python-version")))
        );
    }
    Ok(())
}

#[test]
fn permission_and_unhealthy_errors_retain_failed_source() -> Result<()> {
    let fs = MemoryFs {
        fail_reads: true,
        ..MemoryFs::default()
    };
    let cwd = root();
    let global = cwd.join("global-version");
    let input = request(&cwd, &global);
    let report = resolve(&fs, &input, &[]);
    assert_kind(report.result, ErrorKind::Permission);
    assert_eq!(
        report.steps.last().map(|step| step.outcome),
        Some(TraceOutcome::Rejected)
    );
    let fs = MemoryFs::default();
    fs.insert(&global, b"3.13");
    let mut broken = runtime("3.13.7", 1)?;
    broken.set_health(RuntimeHealth::PendingRemoval);
    let records = [broken];
    let report = resolve(&fs, &input, &records);
    assert_kind(report.result, ErrorKind::BrokenRuntime);
    assert_eq!(
        report.steps.last().map(|step| &step.source),
        Some(&VersionSource::GlobalFile(global))
    );
    Ok(())
}

#[test]
fn system_fallback_requires_trusted_opt_in_and_never_hides_errors() -> Result<()> {
    let fs = MemoryFs::default();
    let cwd = root();
    let global = cwd.join("global-version");
    let mut input = request(&cwd, &global);
    input.explicit = Some("system");
    assert_kind(resolve(&fs, &input, &[]).result, ErrorKind::Usage);
    input.system_fallback = true;
    assert_eq!(
        resolve(&fs, &input, &[]).result?.selection,
        RuntimeSelection::SystemRequested
    );
    input.explicit = None;
    assert_eq!(
        resolve(&fs, &input, &[]).result?.source,
        VersionSource::SystemFallback
    );
    fs.insert(cwd.join(".python-version"), b"system");
    input.system_fallback = false;
    assert_kind(resolve(&fs, &input, &[]).result, ErrorKind::Usage);
    input.system_fallback = true;
    fs.insert(cwd.join(".python-version"), b"3.12");
    assert_kind(resolve(&fs, &input, &[]).result, ErrorKind::NotInstalled);
    Ok(())
}

#[test]
fn writes_pin_short_versions_and_aliases_without_later_drift() -> Result<()> {
    let fs = MemoryFs::default();
    let path = root().join("global-version");
    let mut record = runtime("3.13.7", 1)?;
    record.add_alias("work".parse()?);
    let mut records = vec![record];
    for input in ["3.13", "work", "3.13.7"] {
        let pinned = write_selection_file(&fs, &path, &input.parse()?, &records, false)?;
        assert_eq!(pinned.to_string(), records[0].id().to_string());
        assert_eq!(
            fs.files.borrow().get(&path),
            Some(&format!("{pinned}\n").into_bytes())
        );
    }
    records.push(runtime("3.13.10", 2)?);
    let selector = read_selection_file(&fs, &path)?
        .ok_or_else(|| Error::new(ErrorKind::Internal, "Missing persisted choice"))?;
    assert_eq!(
        selector.select(&records)?,
        RuntimeSelection::Registered(&records[0])
    );
    Ok(())
}

#[test]
fn failed_selection_or_publication_preserves_previous_file() -> Result<()> {
    let fs = MemoryFs::default();
    let path = root().join(".python-version");
    fs.insert(&path, b"old\n");
    let mut records = vec![runtime("3.13.7", 1)?, runtime("3.13.7", 2)?];
    assert_kind(
        write_selection_file(&fs, &path, &"3.13".parse()?, &records, false),
        ErrorKind::Conflict,
    );
    assert_kind(
        write_selection_file(&fs, &path, &"3.12".parse()?, &records, false),
        ErrorKind::NotInstalled,
    );
    assert_kind(
        write_selection_file(&fs, &path, &"system".parse()?, &records, false),
        ErrorKind::Usage,
    );
    records.truncate(1);
    records[0].set_health(RuntimeHealth::Unchecked);
    assert_kind(
        write_selection_file(&fs, &path, &"3.13".parse()?, &records, false),
        ErrorKind::BrokenRuntime,
    );
    assert_eq!(
        fs.files.borrow().get(&path).map(Vec::as_slice),
        Some(b"old\n".as_slice())
    );
    let fs = MemoryFs {
        fail_writes: true,
        ..fs
    };
    assert_kind(
        write_selection_file(&fs, &path, &"system".parse()?, &[], true),
        ErrorKind::Busy,
    );
    assert_eq!(
        fs.files.borrow().get(&path).map(Vec::as_slice),
        Some(b"old\n".as_slice())
    );
    Ok(())
}

#[cfg(feature = "config-toml")]
mod configuration {
    use super::*;
    use pyrudder_core::config::{ConfigEnvironment, ConfigOverrides, file::load_configuration};

    fn environment() -> ConfigEnvironment {
        ConfigEnvironment {
            local_app_data: Some(root().join("local")),
            roaming_app_data: Some(root().join("roaming")),
            ..ConfigEnvironment::default()
        }
    }

    #[test]
    fn missing_config_uses_independent_windows_defaults_without_writes() -> Result<()> {
        let fs = MemoryFs::default();
        let config = load_configuration(&fs, &environment(), &ConfigOverrides::default())?;
        let base = root().join("local/PyRudder");
        assert_eq!(config.paths.install_dir, base.join("bin"));
        assert_eq!(config.paths.runtimes_dir, base.join("runtimes"));
        assert_eq!(config.paths.downloads_dir, base.join("downloads"));
        assert_eq!(config.paths.cache_dir, base.join("cache"));
        assert_eq!(config.paths.temp_dir, base.join("temp"));
        assert_eq!(config.paths.shims_dir, base.join("shims"));
        assert_eq!(config.paths.config_dir, root().join("roaming/PyRudder"));
        assert!(!config.system_fallback);
        assert_eq!(config.loaded_file, None);
        assert!(fs.files.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn home_sets_defaults_but_individual_cli_env_and_file_paths_win() -> Result<()> {
        let home = root().join("home");
        let file_path = root().join("file runtime");
        let env_path = root().join("env runtime");
        let cli_path = root().join("cli runtime");
        for mask in 0_u8..8 {
            let fs = MemoryFs::default();
            let mut env = ConfigEnvironment {
                home: Some(home.clone()),
                ..ConfigEnvironment::default()
            };
            let mut cli = ConfigOverrides::default();
            if mask & 1 != 0 {
                fs.insert(
                    home.join("config/config.toml"),
                    format!(
                        "schema_version = 1\n[paths]\nruntimes_dir = '{}'",
                        file_path.display()
                    ),
                );
            }
            if mask & 2 != 0 {
                env.paths.runtimes_dir = Some(env_path.clone());
            }
            if mask & 4 != 0 {
                cli.paths.runtimes_dir = Some(cli_path.clone());
            }
            let config = load_configuration(&fs, &env, &cli)?;
            let expected = if mask & 4 != 0 {
                cli_path.clone()
            } else if mask & 2 != 0 {
                env_path.clone()
            } else if mask & 1 != 0 {
                file_path.clone()
            } else {
                home.join("runtimes")
            };
            assert_eq!(config.paths.runtimes_dir, expected);
            assert_eq!(config.paths.config_dir, home.join("config"));
        }
        Ok(())
    }

    #[test]
    fn config_location_is_bootstrapped_once_and_cli_can_disable_system_policy() -> Result<()> {
        let fs = MemoryFs::default();
        let mut env = environment();
        env.paths.config_dir = Some(root().join("env-config"));
        let mut cli = ConfigOverrides::default();
        let cli_dir = root().join("cli-config");
        cli.paths.config_dir = Some(cli_dir.clone());
        fs.insert(
            cli_dir.join("config.toml"),
            b"schema_version = 1\n[commands]\nsystem_fallback = true",
        );
        let config = load_configuration(&fs, &env, &cli)?;
        assert!(config.system_fallback);
        assert_eq!(config.loaded_file, Some(cli_dir.join("config.toml")));
        assert_eq!(&*fs.reads.borrow(), &[cli_dir.join("config.toml")]);
        cli.system_fallback = Some(false);
        assert!(!load_configuration(&fs, &env, &cli)?.system_fallback);
        Ok(())
    }

    #[test]
    fn invalid_configuration_is_not_silently_ignored() {
        let fs = MemoryFs::default();
        let env = environment();
        let file = root().join("roaming/PyRudder/config.toml");
        let oversized = vec![b'a'; 65_537];
        for bytes in [
            b"".as_slice(),
            b"schema_version = 2",
            b"schema_version = '1'",
            b"schema_version = 1\nschema_version = 1",
            b"schema_version = 1\nunknown = true",
            b"schema_version = 1\n[paths]\nconfig_dir = 'redirect'",
            b"schema_version = 1\n[commands]\ncross_version_fallback = true",
            b"schema_version = 1\n[paths]\ncache_dir = 'relative'",
            b"\xff",
            &oversized,
        ] {
            fs.insert(&file, bytes);
            assert_kind(
                load_configuration(&fs, &env, &ConfigOverrides::default()),
                ErrorKind::Usage,
            );
        }
    }

    #[test]
    fn missing_roots_and_empty_environment_overrides_fail_closed() {
        let fs = MemoryFs::default();
        assert_kind(
            load_configuration(
                &fs,
                &ConfigEnvironment::default(),
                &ConfigOverrides::default(),
            ),
            ErrorKind::Usage,
        );
        let mut env = environment();
        env.paths.runtimes_dir = Some(PathBuf::new());
        assert_kind(
            load_configuration(&fs, &env, &ConfigOverrides::default()),
            ErrorKind::Usage,
        );
        env.paths.runtimes_dir = None;
        env.paths.config_dir = Some(PathBuf::new());
        assert_kind(
            load_configuration(&fs, &env, &ConfigOverrides::default()),
            ErrorKind::Usage,
        );
    }
}
