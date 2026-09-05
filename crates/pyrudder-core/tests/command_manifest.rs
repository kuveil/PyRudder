//! Pure inventory tests; the ASCII-only mapper is a test double, not Windows casing.
//! 纯清单测试；仅处理 ASCII 的映射器是测试替身，不代表 Windows 大小写语义。

use pyrudder_core::{
    ErrorKind, Result,
    commands::{
        CommandCaseMapper, CommandExtension, CommandKey, CommandKind, CommandOrigin,
        CommandRejection, CommandRequest, CommandStatus, CommandTarget, EntryFingerprint,
        RuntimeCommandManifest, command_union, is_reserved, lookup_command,
    },
    runtime::RuntimeId,
};
use std::{path::PathBuf, time::SystemTime};

struct AsciiMapper;
impl CommandCaseMapper for AsciiMapper {
    fn uppercase(&self, name: &str) -> Result<String> {
        Ok(name.to_ascii_uppercase())
    }
}

fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\fake")
    } else {
        PathBuf::from("/fake")
    }
}
fn id(version: &str) -> Result<RuntimeId> {
    RuntimeId::new(version.parse()?)
}
fn bare(name: &str) -> Result<CommandRequest> {
    Ok(CommandRequest::Bare(CommandKey::new(name, &AsciiMapper)?))
}
fn exact(name: &str) -> Result<CommandRequest> {
    Ok(CommandRequest::Exact(CommandKey::new(name, &AsciiMapper)?))
}
fn target(name: &str, origin: CommandOrigin, extension: CommandExtension) -> CommandTarget {
    let directory = match origin {
        CommandOrigin::Root => root(),
        CommandOrigin::Script(index) => root().join(format!("Scripts{index}")),
    };
    CommandTarget {
        path: directory.join(name),
        origin,
        extension,
        status: CommandStatus::Available(match extension {
            CommandExtension::Exe | CommandExtension::Com => CommandKind::PeConsole,
            CommandExtension::Cmd => CommandKind::Cmd,
            CommandExtension::Bat => CommandKind::Bat,
            CommandExtension::PowerShell => CommandKind::PowerShell,
        }),
        fingerprint: EntryFingerprint {
            name: name.into(),
            size: 1024,
            modified: SystemTime::UNIX_EPOCH,
            attributes: 0x20,
        },
    }
}
fn manifest(version: &str, targets: Vec<CommandTarget>) -> Result<RuntimeCommandManifest> {
    RuntimeCommandManifest::build(id(version)?, targets, Vec::new(), Vec::new(), &AsciiMapper)
}

#[test]
fn rejects_paths_devices_controls_and_lossy_windows_names() {
    for name in [
        "",
        ".",
        "..",
        "a/b",
        r"a\b",
        "a:stream",
        "x\0",
        "x\n",
        "x\u{7f}",
        "x\u{85}",
        "x?",
        "x*",
        "x|",
        "x\"",
        "x<",
        "x>",
        "x.",
        "x ",
        "NUL",
        "CON.txt",
        "COM1",
        "lpt².exe",
        "conin$",
        "con .cmd",
    ] {
        assert!(CommandKey::new(name, &AsciiMapper).is_err(), "{name:?}");
    }
}

#[test]
fn validates_utf16_limits_without_rejecting_spaces_or_inner_dots() -> Result<()> {
    for name in [
        "pip3.13",
        "hello world",
        "工作台",
        "com10",
        "auxiliary",
        " leading",
    ] {
        CommandKey::new(name, &AsciiMapper)?;
    }
    CommandKey::new(&"a".repeat(255), &AsciiMapper)?;
    assert!(CommandKey::new(&"a".repeat(256), &AsciiMapper).is_err());
    CommandKey::new(&"🦀".repeat(127), &AsciiMapper)?;
    assert!(CommandKey::new(&"🦀".repeat(128), &AsciiMapper).is_err());
    Ok(())
}

#[test]
fn reserves_manager_namespace_and_official_launcher_only() {
    for name in [
        "pyrudder",
        "PyRudder-shim-console",
        "pyrudder-dispatcher",
        "pyrudder.exe",
        "PY",
        "py.exe",
    ] {
        assert!(is_reserved(name));
    }
    for name in ["python", "pythonw", "pytest", "pyrudderx", "pip3.13"] {
        assert!(!is_reserved(name));
    }
}

#[test]
fn extension_order_is_explicit_and_uses_only_last_suffix() {
    use CommandExtension::{Bat, Cmd, Com, Exe, PowerShell};
    assert!(Exe < Com && Com < Cmd && Cmd < Bat && Bat < PowerShell);
    assert_eq!(
        CommandExtension::split("pip3.13.EXE"),
        Some(("pip3.13", Exe))
    );
    assert_eq!(CommandExtension::split("x.exe.cmd"), Some(("x.exe", Cmd)));
    for name in ["x.dll", "x.pyd", "x.py", "x", "x.exe."] {
        assert!(CommandExtension::split(name).is_none());
    }
}

#[test]
fn priority_is_source_then_extension_and_all_conflicts_are_retained() -> Result<()> {
    let candidates = vec![
        target("tool.exe", CommandOrigin::Script(1), CommandExtension::Exe),
        target("TOOL.cmd", CommandOrigin::Root, CommandExtension::Cmd),
        target("tool.exe", CommandOrigin::Script(0), CommandExtension::Exe),
        target("tool.bat", CommandOrigin::Root, CommandExtension::Bat),
    ];
    let inventory = manifest("3.13.7", candidates.clone())?;
    let reversed = manifest("3.13.7", candidates.into_iter().rev().collect())?;
    assert_eq!(inventory, reversed);
    let entry = &inventory.commands()[&bare("Tool")?];
    assert_eq!(entry.winner.path, root().join("TOOL.cmd"));
    assert_eq!(entry.shadowed.len(), 3);
    assert_eq!(entry.shadowed[0].extension, CommandExtension::Bat);
    assert_eq!(entry.shadowed[1].origin, CommandOrigin::Script(0));
    assert_eq!(
        inventory.lookup(&exact("TOOL.exe")?)?.origin,
        CommandOrigin::Script(0)
    );
    Ok(())
}

#[test]
fn same_source_extensions_and_case_ties_are_deterministic() -> Result<()> {
    let candidates = vec![
        target("tool.exe", CommandOrigin::Root, CommandExtension::Exe),
        target("TOOL.EXE", CommandOrigin::Root, CommandExtension::Exe),
        target("tool.com", CommandOrigin::Root, CommandExtension::Com),
        target(
            "tool.ps1",
            CommandOrigin::Root,
            CommandExtension::PowerShell,
        ),
    ];
    let first = manifest("3.13.7", candidates.clone())?;
    assert_eq!(
        first,
        manifest("3.13.7", candidates.into_iter().rev().collect())?
    );
    assert_eq!(first.lookup(&bare("tool")?)?.path, root().join("TOOL.EXE"));
    assert_eq!(first.commands()[&exact("tool.exe")?].shadowed.len(), 1);
    Ok(())
}

#[test]
fn dotted_stems_do_not_overwrite_explicit_filename_requests() -> Result<()> {
    let inventory = manifest(
        "3.13.7",
        vec![
            target("foo.exe", CommandOrigin::Root, CommandExtension::Exe),
            target("foo.exe.cmd", CommandOrigin::Root, CommandExtension::Cmd),
        ],
    )?;
    assert_eq!(
        inventory.lookup(&bare("foo.exe")?)?.path,
        root().join("foo.exe.cmd")
    );
    assert_eq!(
        inventory.lookup(&exact("foo.exe")?)?.path,
        root().join("foo.exe")
    );
    assert!(inventory.lookup(&exact("foo")?).is_err());
    Ok(())
}

#[test]
fn rejected_winner_blocks_lower_candidates_but_not_explicit_other_extensions() -> Result<()> {
    let mut bad = target("tool.exe", CommandOrigin::Root, CommandExtension::Exe);
    bad.status = CommandStatus::Rejected(CommandRejection::InvalidPe);
    let inventory = manifest(
        "3.13.7",
        vec![
            bad,
            target("tool.cmd", CommandOrigin::Root, CommandExtension::Cmd),
        ],
    )?;
    assert_eq!(
        inventory.lookup(&bare("tool")?).err().map(|e| e.kind()),
        Some(ErrorKind::BrokenRuntime)
    );
    assert!(inventory.lookup(&exact("tool.cmd")?).is_ok());
    let union = command_union(&[inventory])?;
    assert!(!union.contains_key(&bare("tool")?));
    assert!(union.contains_key(&exact("tool.cmd")?));
    Ok(())
}

#[test]
fn union_is_order_independent_and_missing_commands_never_cross_runtimes() -> Result<()> {
    let first = manifest("3.13.7", Vec::new())?;
    let second = manifest(
        "3.12.10",
        vec![target(
            "pytest.exe",
            CommandOrigin::Root,
            CommandExtension::Exe,
        )],
    )?;
    let inventories = vec![first.clone(), second.clone()];
    assert_eq!(
        command_union(&inventories)?,
        command_union(&[second, first])?
    );
    assert_eq!(
        lookup_command(&inventories, &id("3.13.7")?, &bare("pytest")?)
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::CommandMissing)
    );
    assert!(lookup_command(&inventories, &id("3.12.10")?, &bare("pytest")?).is_ok());
    assert_eq!(
        lookup_command(&inventories, &id("3.11.9")?, &bare("pytest")?)
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::NotInstalled)
    );
    Ok(())
}

#[test]
fn duplicate_runtime_snapshots_are_not_silently_chosen() -> Result<()> {
    let inventory = manifest("3.13.7", Vec::new())?;
    let duplicates = [inventory.clone(), inventory];
    assert_eq!(
        command_union(&duplicates).err().map(|e| e.kind()),
        Some(ErrorKind::Conflict)
    );
    assert_eq!(
        lookup_command(&duplicates, &id("3.13.7")?, &bare("python")?)
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::Conflict)
    );
    Ok(())
}

#[test]
fn builder_rejects_invalid_reserved_and_inconsistent_targets() {
    for name in ["pyrudder.exe", "py.exe", ".exe", "CON.exe", "x.dll"] {
        assert!(
            manifest(
                "3.13.7",
                vec![target(name, CommandOrigin::Root, CommandExtension::Exe)]
            )
            .is_err()
        );
    }
    let mut candidate = target("x.exe", CommandOrigin::Root, CommandExtension::Cmd);
    assert!(manifest("3.13.7", vec![candidate.clone()]).is_err());
    candidate.extension = CommandExtension::Exe;
    candidate.path = PathBuf::from("x.exe");
    assert!(manifest("3.13.7", vec![candidate.clone()]).is_err());
    candidate.path = root().join("..").join("x.exe");
    assert!(manifest("3.13.7", vec![candidate]).is_err());
}

#[test]
fn builder_rejects_duplicate_observations_and_mismatched_metadata_or_kind() {
    let mut candidate = target("x.exe", CommandOrigin::Root, CommandExtension::Exe);
    assert_eq!(
        manifest("3.13.7", vec![candidate.clone(), candidate.clone()])
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::Conflict)
    );
    candidate.fingerprint.name = "y.exe".into();
    assert!(manifest("3.13.7", vec![candidate.clone()]).is_err());
    candidate.fingerprint.name = "x.exe".into();
    candidate.status = CommandStatus::Available(CommandKind::Cmd);
    assert!(manifest("3.13.7", vec![candidate]).is_err());
}
