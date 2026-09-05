//! Native Windows filesystem integration; every write stays in a test-owned temporary tree.
//! 原生 Windows 文件系统集成；所有写入仅发生在测试拥有的临时目录树。

#![cfg(windows)]

use std::{
    fs, io,
    os::windows::{
        ffi::OsStringExt,
        fs::{OpenOptionsExt, symlink_dir, symlink_file},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Barrier,
    thread,
};

use pyrudder_core::{
    ErrorKind,
    config::{ConfigEnvironment, ConfigOverrides, file::load_configuration},
    resolver::{ResolveRequest, VersionSource, resolve},
    runtime::{InstallationId, RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord},
    selector::RuntimeSelection,
    state::{StateFileSystem, parse_selection_file, read_selection_file, write_selection_file},
};
use pyrudder_platform_windows::state::WindowsStateFileSystem;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn temporary() -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("pyrudder-state-tests-")
        .tempdir()
}

fn record(root: &Path, version: &str, installation: u128) -> pyrudder_core::Result<RuntimeRecord> {
    let id =
        RuntimeId::new(version.parse()?)?.with_installation(InstallationId::new(installation)?);
    let mut record = RuntimeRecord::new(
        id,
        root.to_path_buf(),
        vec![root.join("Scripts")],
        RuntimeOrigin::External {
            registered_executable: root.join("python.exe"),
        },
    )?;
    record.set_health(RuntimeHealth::Ready);
    Ok(record)
}

#[test]
fn missing_files_are_absent_but_directories_and_oversized_files_are_errors() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    assert_eq!(fs.read_file(&temp.path().join("missing/file"), 10)?, None);
    assert!(fs.read_file(temp.path(), 10).is_err());
    let file = temp.path().join("data");
    fs::write(&file, [b'a'; 10])?;
    assert_eq!(fs.read_file(&file, 10)?, Some(vec![b'a'; 10]));
    assert!(matches!(fs.read_file(&file, 9), Err(error) if error.kind() == ErrorKind::Usage));
    assert!(fs.read_file(&file.join("child"), 10).is_err());
    Ok(())
}

#[test]
fn normalization_preserves_unicode_spaces_and_missing_tails_without_creating_them() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let existing = temp.path().join("团队 Python");
    fs::create_dir(&existing)?;
    let path = existing.join("./future/versions");
    let normalized = fs.normalize_directory(&path)?;
    assert_eq!(
        normalized,
        fs::canonicalize(&existing)?.join("future/versions")
    );
    assert!(!existing.join("future").exists());
    assert!(fs.canonical_directory(&path).is_err());
    Ok(())
}

#[test]
fn windows_ambiguous_and_device_paths_are_rejected_before_io() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    for path in [
        PathBuf::from(r"C:relative"),
        PathBuf::from(r"\root-relative"),
        PathBuf::from(r"\\server\share\dir"),
        PathBuf::from(r"\\.\pipe\name"),
        temp.path().join("../other"),
        temp.path().join("NUL.txt"),
        temp.path().join("COM¹.log"),
        temp.path().join("space "),
        temp.path().join("dot."),
        temp.path().join("file:stream"),
        temp.path().join("a\0b"),
    ] {
        assert!(
            matches!(fs.normalize_directory(&path), Err(error) if error.kind() == ErrorKind::Usage)
        );
        assert!(fs.write_atomic(&path, b"unchanged").is_err());
    }
    assert_eq!(fs::read_dir(temp.path())?.count(), 0);
    Ok(())
}

#[test]
fn native_config_defaults_and_explicit_config_directory_are_normalized() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let mut env = ConfigEnvironment {
        home: Some(temp.path().to_path_buf()),
        ..ConfigEnvironment::default()
    };
    let config = load_configuration(&fs, &env, &ConfigOverrides::default())?;
    assert_eq!(
        config.paths.config_dir,
        fs::canonicalize(temp.path())?.join("config")
    );
    assert_eq!(fs::read_dir(temp.path())?.count(), 0);
    let config_dir = temp.path().join("设置");
    fs::create_dir(&config_dir)?;
    fs::write(
        config_dir.join("config.toml"),
        "\u{feff}schema_version = 1\n[commands]\nsystem_fallback = true\n",
    )?;
    env.paths.config_dir = Some(config_dir.clone());
    let config = load_configuration(&fs, &env, &ConfigOverrides::default())?;
    assert!(config.system_fallback);
    assert_eq!(
        config.loaded_file,
        Some(fs::canonicalize(config_dir)?.join("config.toml"))
    );
    Ok(())
}

#[test]
fn atomic_creation_replacement_and_pinning_work_in_unicode_directories() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let directory = temp.path().join("中文 工作区");
    fs::create_dir(&directory)?;
    let global = directory.join("global-version");
    let local = directory.join(".python-version");
    let records = [record(temp.path(), "3.13.7", 1)?];
    let pinned = write_selection_file(&fs, &global, &"3.13".parse()?, &records, false)?;
    assert_eq!(fs::read(&global)?, format!("{pinned}\n").as_bytes());
    assert_eq!(
        read_selection_file(&fs, &global)?.map(|value| value.to_string()),
        Some(pinned.to_string())
    );
    write_selection_file(&fs, &local, &"3.13".parse()?, &records, false)?;
    write_selection_file(&fs, &global, &"system".parse()?, &[], true)?;
    assert_eq!(fs::read(&global)?, b"system\n");
    assert_eq!(fs::read_dir(&directory)?.count(), 2);
    Ok(())
}

#[test]
fn locked_destination_keeps_old_bytes_and_cleans_staging() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let file = temp.path().join("global-version");
    fs.write_atomic(&file, b"3.13\n")?;
    // Deny delete sharing to force native publication failure, not a mock failure.
    // 禁止共享删除，触发原生发布失败，而不是模拟错误。
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&file)?;
    assert!(
        matches!(fs.write_atomic(&file, b"3.12\n"), Err(error) if matches!(error.kind(), ErrorKind::Busy | ErrorKind::Permission))
    );
    assert_eq!(fs::read(&file)?, b"3.13\n");
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    drop(lock);
    fs.write_atomic(&file, b"3.12\n")?;
    assert_eq!(fs::read(&file)?, b"3.12\n");
    Ok(())
}

#[test]
fn readonly_and_directory_destinations_are_not_replaced() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let file = temp.path().join("global-version");
    fs::write(&file, b"old\n")?;
    let original_permissions = fs::metadata(&file)?.permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&file, readonly)?;
    let result = fs.write_atomic(&file, b"new\n");
    fs::set_permissions(&file, original_permissions)?;
    assert!(matches!(result, Err(error) if error.kind() == ErrorKind::Permission));
    assert_eq!(fs::read(&file)?, b"old\n");
    let directory = temp.path().join("directory");
    fs::create_dir(&directory)?;
    assert!(fs.write_atomic(&directory, b"new").is_err());
    assert!(directory.is_dir());
    let absent = temp.path().join("absent/global-version");
    assert!(fs.write_atomic(&absent, b"new").is_err());
    assert!(!temp.path().join("absent").exists());
    Ok(())
}

#[test]
fn concurrent_readers_observe_only_complete_old_or_new_snapshots() -> TestResult {
    let temp = temporary()?;
    let fs = WindowsStateFileSystem;
    let path = temp.path().join("global-version");
    let old = b"cpython-3.13.7-x64@00000000000000000000000000000001\n";
    let new = b"cpython-3.13.10-x64@00000000000000000000000000000002\n";
    fs.write_atomic(&path, old)?;
    let barrier = Barrier::new(4);
    thread::scope(|scope| -> TestResult {
        let mut writers = Vec::new();
        for _ in 0..3 {
            writers.push(scope.spawn(|| -> pyrudder_core::Result<()> {
                barrier.wait();
                for index in 0..30 {
                    fs.write_atomic(&path, if index % 2 == 0 { old } else { new })?;
                }
                Ok(())
            }));
        }
        barrier.wait();
        for _ in 0..200 {
            let bytes = fs
                .read_file(&path, 133)?
                .ok_or_else(|| io::Error::other("Atomic file disappeared"))?;
            assert!(bytes == old || bytes == new);
            parse_selection_file(&bytes)?;
        }
        for writer in writers {
            writer
                .join()
                .map_err(|_| io::Error::other("Writer panicked"))??;
        }
        Ok(())
    })?;
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    Ok(())
}

#[test]
fn independent_processes_can_publish_complete_snapshots() -> TestResult {
    let temp = temporary()?;
    let path = temp.path().join("global-version");
    WindowsStateFileSystem.write_atomic(&path, b"3.13\n")?;
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            Command::new(std::env::current_exe()?)
                .args(["--ignored", "--exact", "atomic_writer_process_worker"])
                .env("PYRUDDER_STATE_TEST_OUTPUT", &path)
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()?,
        );
    }
    for _ in 0..100 {
        let bytes = WindowsStateFileSystem
            .read_file(&path, 133)?
            .ok_or_else(|| io::Error::other("Missing snapshot"))?;
        assert!(bytes == b"3.13\n" || bytes == b"3.12\n");
    }
    let outputs = children
        .into_iter()
        .map(std::process::Child::wait_with_output)
        .collect::<io::Result<Vec<_>>>()?;
    for output in outputs {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    Ok(())
}

#[test]
#[ignore = "Launched explicitly by the parent integration test. / 由父集成测试显式启动。"]
fn atomic_writer_process_worker() -> TestResult {
    let path = PathBuf::from(
        std::env::var_os("PYRUDDER_STATE_TEST_OUTPUT")
            .ok_or_else(|| io::Error::other("Missing test target"))?,
    );
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .ok_or_else(|| io::Error::other("Invalid test target"))?;
    assert!(
        parent
            .to_string_lossy()
            .starts_with("pyrudder-state-tests-")
    );
    assert_eq!(
        path.file_name(),
        Some(std::ffi::OsStr::new("global-version"))
    );
    for index in 0..20 {
        WindowsStateFileSystem
            .write_atomic(&path, if index % 2 == 0 { b"3.13\n" } else { b"3.12\n" })?;
    }
    Ok(())
}

#[test]
fn native_project_resolution_respects_case_normalized_inclusive_boundary() -> TestResult {
    let temp = temporary()?;
    let workspace = temp.path().join("MiXeD 工作区");
    let cwd = workspace.join("project/src");
    fs::create_dir_all(&cwd)?;
    let global = temp.path().join("global-version");
    fs::write(temp.path().join(".python-version"), b"outside-invalid")?;
    let records = [record(temp.path(), "3.13.7", 1)?];
    write_selection_file(
        &WindowsStateFileSystem,
        &workspace.join(".python-version"),
        &"3.13".parse()?,
        &records,
        false,
    )?;
    let case_boundary = PathBuf::from(workspace.to_string_lossy().to_uppercase());
    let input = ResolveRequest {
        explicit: None,
        shell_version: None,
        cwd: &cwd,
        workspace_boundary: Some(&case_boundary),
        global_file: &global,
        system_fallback: false,
    };
    let report = resolve(&WindowsStateFileSystem, &input, &records);
    let selected = report.result?;
    assert_eq!(
        selected.selection,
        RuntimeSelection::Registered(&records[0])
    );
    assert_eq!(
        selected.source,
        VersionSource::LocalFile(fs::canonicalize(&workspace)?.join(".python-version"))
    );
    Ok(())
}

#[test]
fn nonunicode_shell_selection_is_rejected_only_when_reached() -> TestResult {
    let temp = temporary()?;
    let global = temp.path().join("global-version");
    let invalid = std::ffi::OsString::from_wide(&[0xd800]);
    let mut input = ResolveRequest {
        explicit: None,
        shell_version: Some(&invalid),
        cwd: temp.path(),
        workspace_boundary: Some(temp.path()),
        global_file: &global,
        system_fallback: false,
    };
    assert!(
        matches!(resolve(&WindowsStateFileSystem, &input, &[]).result, Err(error) if error.kind() == ErrorKind::Usage)
    );
    input.explicit = Some("3.13");
    assert!(
        resolve(
            &WindowsStateFileSystem,
            &input,
            &[record(temp.path(), "3.13.7", 1)?]
        )
        .result
        .is_ok()
    );
    Ok(())
}

#[test]
#[ignore = "Requires Windows symlink privilege; junction coverage runs by default. / 需要 Windows 符号链接权限；默认执行 junction 覆盖。"]
fn symlink_files_and_ancestor_directories_are_rejected() -> TestResult {
    let temp = temporary()?;
    let target = temp.path().join("actual");
    fs::create_dir(&target)?;
    fs::write(target.join("selection"), b"3.13\n")?;
    let link = temp.path().join("link");
    symlink_dir(&target, &link)?;
    let file_link = temp.path().join("file-link");
    symlink_file(target.join("selection"), &file_link)?;
    let fs = WindowsStateFileSystem;
    assert!(fs.read_file(&link.join("selection"), 133).is_err());
    assert!(fs.read_file(&file_link, 133).is_err());
    assert!(fs.write_atomic(&file_link, b"3.12\n").is_err());
    assert!(fs.normalize_directory(&link.join("future")).is_err());
    assert_eq!(fs::read(target.join("selection"))?, b"3.13\n");
    Ok(())
}

#[test]
fn junctions_cannot_redirect_reads_writes_or_workspace_boundaries() -> TestResult {
    let temp = temporary()?;
    let target = temp.path().join("actual");
    fs::create_dir(&target)?;
    fs::write(target.join(".python-version"), b"3.13\n")?;
    let link = temp.path().join("junction");
    junction::create(&target, &link)?;
    let fs = WindowsStateFileSystem;
    assert!(fs.read_file(&link.join(".python-version"), 133).is_err());
    assert!(
        fs.write_atomic(&link.join(".python-version"), b"3.12\n")
            .is_err()
    );
    assert!(fs.normalize_directory(&link.join("future")).is_err());
    assert!(fs.canonical_directory(&link).is_err());
    let global = temp.path().join("global-version");
    let input = ResolveRequest {
        explicit: None,
        shell_version: None,
        cwd: &link,
        workspace_boundary: Some(temp.path()),
        global_file: &global,
        system_fallback: true,
    };
    assert!(
        matches!(resolve(&fs, &input, &[]).result, Err(error) if error.kind() == ErrorKind::Usage)
    );
    assert_eq!(fs::read(target.join(".python-version"))?, b"3.13\n");
    Ok(())
}

#[test]
fn replacing_a_hardlinked_selection_does_not_modify_the_other_file() -> TestResult {
    let temp = temporary()?;
    let original = temp.path().join("original");
    let selection = temp.path().join(".python-version");
    fs::write(&original, b"3.13\n")?;
    fs::hard_link(&original, &selection)?;
    WindowsStateFileSystem.write_atomic(&selection, b"3.12\n")?;
    assert_eq!(fs::read(&original)?, b"3.13\n");
    assert_eq!(fs::read(&selection)?, b"3.12\n");
    Ok(())
}
