//! Native discovery integration tests, isolated to disposable fake runtime directories.
//! 原生命令发现集成测试，隔离在可丢弃的假运行时目录中。

#![cfg(windows)]

#[path = "../../../tests/fixtures/fake_runtime.rs"]
mod fixtures;

use fixtures::{FakeRuntime, TestResult, pe_header};
use pyrudder_core::{
    ErrorKind, Result,
    commands::{
        CommandCaseMapper, CommandExtension, CommandKey, CommandKind, CommandOrigin,
        CommandRejection, CommandRequest, CommandStatus, DiscoveryIssue, command_union,
        lookup_command,
    },
    runtime::{RuntimeOrigin, RuntimeRecord},
};
use pyrudder_platform_windows::commands::{
    DiscoveryLimits, WindowsCommandCaseMapper, discover_commands, discover_commands_with_limits,
};
use std::{
    ffi::OsString,
    fs,
    os::windows::{ffi::OsStringExt, fs::OpenOptionsExt},
};

fn bare(name: &str) -> Result<CommandRequest> {
    Ok(CommandRequest::Bare(CommandKey::new(
        name,
        &WindowsCommandCaseMapper,
    )?))
}
fn exact(name: &str) -> Result<CommandRequest> {
    Ok(CommandRequest::Exact(CommandKey::new(
        name,
        &WindowsCommandCaseMapper,
    )?))
}

#[test]
fn discovers_unknown_direct_children_all_supported_types_and_nothing_recursive() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    for (file, subsystem) in [
        ("python.exe", 3),
        ("pythonw.exe", 2),
        ("Scripts/mystery.COM", 3),
        ("Scripts/editor.com", 2),
    ] {
        runtime.write(file, pe_header(subsystem))?;
    }
    for file in [
        "Scripts/custom.cmd",
        "Scripts/task.bat",
        "Scripts/runner.ps1",
    ] {
        runtime.write(file, b"inert fixture")?;
    }
    for file in [
        "python.dll",
        "module.pyd",
        "module.py",
        "Scripts/extensionless",
    ] {
        runtime.write(file, pe_header(3))?;
    }
    fs::create_dir_all(runtime.record.root().join("Lib/site-packages"))?;
    runtime.write("Lib/site-packages/hidden.exe", pe_header(3))?;
    fs::create_dir(runtime.record.root().join("Scripts/nested"))?;
    runtime.write("Scripts/nested/deep.cmd", b"inert")?;
    fs::create_dir(runtime.record.root().join("directory.exe"))?;
    let inventory = discover_commands(&runtime.record)?;
    assert_eq!(inventory.commands().len(), 14);
    for (name, kind) in [
        ("python", CommandKind::PeConsole),
        ("pythonw", CommandKind::PeGui),
        ("mystery", CommandKind::PeConsole),
        ("editor", CommandKind::PeGui),
        ("custom", CommandKind::Cmd),
        ("task", CommandKind::Bat),
        ("runner", CommandKind::PowerShell),
    ] {
        assert_eq!(
            inventory.lookup(&bare(name)?)?.status,
            CommandStatus::Available(kind)
        );
    }
    assert_eq!(
        inventory.lookup(&exact("MYSTERY.com")?)?.extension,
        CommandExtension::Com
    );
    assert!(inventory.diagnostics().is_empty());
    assert_eq!(inventory, discover_commands(&runtime.record)?);
    Ok(())
}

#[test]
fn unicode_casing_is_native_non_linguistic_and_preserves_distinct_normalizations() -> TestResult {
    for (left, right) in [
        ("pip", "PIP"),
        ("Ärger", "ÄRGER"),
        ("σ", "Σ"),
        ("проба", "ПРОБА"),
    ] {
        assert_eq!(
            CommandKey::new(left, &WindowsCommandCaseMapper)?,
            CommandKey::new(right, &WindowsCommandCaseMapper)?
        );
    }
    for (left, right) in [
        ("ς", "Σ"),
        ("ı", "I"),
        ("𐐀", "𐐨"),
        ("straße", "STRASSE"),
        ("é", "e\u{301}"),
        ("İ", "I"),
        ("Ａ", "A"),
    ] {
        assert_ne!(
            CommandKey::new(left, &WindowsCommandCaseMapper)?,
            CommandKey::new(right, &WindowsCommandCaseMapper)?
        );
    }
    for text in ["", "nul\0tail", &"a".repeat(32_768)] {
        assert!(WindowsCommandCaseMapper.uppercase(text).is_err());
    }
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    runtime.write("Ärger.cmd", b"inert")?;
    runtime.write("Scripts/ärger.exe", pe_header(3))?;
    runtime.write("Scripts/工作 台.ps1", b"inert")?;
    let inventory = discover_commands(&runtime.record)?;
    assert_eq!(inventory.commands()[&bare("ÄRGER")?].shadowed.len(), 1);
    assert_eq!(
        inventory.lookup(&bare("ärger")?)?.origin,
        CommandOrigin::Root
    );
    assert!(inventory.lookup(&bare("工作 台")?).is_ok());
    Ok(())
}

#[test]
fn root_script_order_and_explicit_extensions_have_independent_priority() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    fs::create_dir(runtime.record.root().join("Other"))?;
    let mut record = runtime.record.clone();
    record = RuntimeRecord::new(
        record.id().clone(),
        record.root().into(),
        vec![record.root().join("Other"), record.root().join("Scripts")],
        record.origin().clone(),
    )?;
    runtime.write("tool.bat", b"inert")?;
    runtime.write("Scripts/tool.exe", pe_header(3))?;
    runtime.write("Other/TOOL.cmd", b"inert")?;
    runtime.write("Other/tool.exe", pe_header(2))?;
    let inventory = discover_commands(&record)?;
    assert_eq!(
        inventory.lookup(&bare("tool")?)?.extension,
        CommandExtension::Bat
    );
    assert_eq!(inventory.commands()[&bare("tool")?].shadowed.len(), 3);
    assert_eq!(
        inventory.lookup(&exact("tool.exe")?)?.origin,
        CommandOrigin::Script(0)
    );
    assert_eq!(
        inventory.lookup(&exact("tool.exe")?)?.status,
        CommandStatus::Available(CommandKind::PeGui)
    );
    Ok(())
}

#[test]
fn reserved_names_and_invalid_unicode_are_diagnostic_not_commands() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    for name in [
        "py.exe",
        "PYRUDDER.cmd",
        "pyrudder-dispatcher.exe",
        "pyrudder-shim-gui.exe",
        "py.exe.cmd",
    ] {
        runtime.write(name, b"not read")?;
    }
    let name = OsString::from_wide(&[0xd800, 46, 99, 109, 100]);
    runtime.write(name, b"not read")?;
    runtime.write(".exe", b"not read")?;
    let inventory = discover_commands(&runtime.record)?;
    assert!(inventory.commands().is_empty());
    assert_eq!(
        inventory
            .diagnostics()
            .iter()
            .filter(|d| d.reason == DiscoveryIssue::ReservedName)
            .count(),
        5
    );
    assert_eq!(
        inventory
            .diagnostics()
            .iter()
            .filter(|d| d.reason == DiscoveryIssue::InvalidName)
            .count(),
        2
    );
    Ok(())
}

#[test]
fn rejected_exe_never_falls_back_to_valid_script_or_other_runtime() -> TestResult {
    let first = FakeRuntime::new("3.13.7", 1)?;
    let second = FakeRuntime::new("3.13.7", 2)?;
    first.write("tool.exe", b"not a PE")?;
    first.write("Scripts/tool.exe", pe_header(3))?;
    first.write("tool.cmd", b"inert")?;
    first.write("legacy.com", [0xb8, 0, 0, 0xcd, 0x20])?;
    second.write("tool.exe", pe_header(3))?;
    second.write("pytest.exe", pe_header(3))?;
    let inventories = [
        discover_commands(&first.record)?,
        discover_commands(&second.record)?,
    ];
    assert_eq!(
        lookup_command(&inventories, first.record.id(), &bare("tool")?)
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::BrokenRuntime)
    );
    assert_eq!(
        lookup_command(&inventories, first.record.id(), &bare("pytest")?)
            .err()
            .map(|e| e.kind()),
        Some(ErrorKind::CommandMissing)
    );
    assert!(inventories[0].lookup(&exact("tool.cmd")?).is_ok());
    assert_eq!(
        inventories[0].commands()[&bare("legacy")?].winner.status,
        CommandStatus::Rejected(CommandRejection::UnsupportedImage)
    );
    assert_eq!(command_union(&inventories)?[&bare("tool")?].len(), 1);
    Ok(())
}

#[test]
fn malformed_headers_dll_architecture_and_subsystems_are_rejected() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    let mut variants = Vec::new();
    for (offset, bytes) in [
        (60, u32::MAX.to_le_bytes().to_vec()),
        (128, b"NE\0\0".to_vec()),
        (132, 0x14c_u16.to_le_bytes().to_vec()),
        (132, 0xaa64_u16.to_le_bytes().to_vec()),
        (134, 97_u16.to_le_bytes().to_vec()),
        (148, 70_u16.to_le_bytes().to_vec()),
        (150, 0x2022_u16.to_le_bytes().to_vec()),
        (150, 0_u16.to_le_bytes().to_vec()),
        (152, 0x10b_u16.to_le_bytes().to_vec()),
        (220, 1_u16.to_le_bytes().to_vec()),
        (260, u32::MAX.to_le_bytes().to_vec()),
    ] {
        let mut image = pe_header(3);
        image[offset..offset + bytes.len()].copy_from_slice(&bytes);
        variants.push(image);
    }
    for length in [0, 63, 128, 151, 221, 263, 431] {
        variants.push(pe_header(3)[..length].to_vec());
    }
    for (index, bytes) in variants.iter().enumerate() {
        runtime.write(format!("bad{index}.exe"), bytes)?;
    }
    let inventory = discover_commands(&runtime.record)?;
    assert_eq!(inventory.diagnostics().len(), variants.len());
    assert!(
        inventory
            .commands()
            .values()
            .all(|entry| matches!(entry.winner.status, CommandStatus::Rejected(_)))
    );
    assert!(command_union(&[inventory])?.is_empty());
    Ok(())
}

#[test]
fn directory_fingerprints_detect_add_remove_rename_and_size_changes() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    let before = discover_commands(&runtime.record)?;
    let path = runtime.write("Scripts/tool.cmd", b"one")?;
    let added = discover_commands(&runtime.record)?;
    assert_ne!(before.directories(), added.directories());
    fs::write(&path, b"changed and longer")?;
    let changed = discover_commands(&runtime.record)?;
    assert_ne!(added.directories(), changed.directories());
    let renamed = path.with_file_name("renamed.cmd");
    fs::rename(&path, &renamed)?;
    let moved = discover_commands(&runtime.record)?;
    assert_ne!(changed.directories(), moved.directories());
    assert!(moved.lookup(&bare("tool")?).is_err());
    assert!(moved.lookup(&bare("renamed")?).is_ok());
    fs::remove_file(renamed)?;
    assert!(discover_commands(&runtime.record)?.commands().is_empty());
    Ok(())
}

#[test]
fn missing_optional_directory_is_recorded_but_missing_root_is_an_error() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    fs::remove_dir(runtime.record.root().join("Scripts"))?;
    let inventory = discover_commands(&runtime.record)?;
    assert_eq!(inventory.directories()[1].modified, None);
    assert_eq!(
        inventory.diagnostics()[0].reason,
        DiscoveryIssue::MissingDirectory
    );
    fs::create_dir(runtime.record.root().join("Scripts"))?;
    assert_ne!(
        inventory.directories(),
        discover_commands(&runtime.record)?.directories()
    );
    let root = runtime.record.root().join("missing");
    let record = RuntimeRecord::new(
        runtime.record.id().clone(),
        root.clone(),
        vec![root.join("Scripts")],
        RuntimeOrigin::External {
            registered_executable: root.join("python.exe"),
        },
    )?;
    assert_eq!(
        discover_commands(&record).err().map(|e| e.kind()),
        Some(ErrorKind::NotInstalled)
    );
    Ok(())
}

#[test]
fn equivalent_directories_are_scanned_only_once() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    runtime.write("tool.cmd", b"inert")?;
    runtime.write("Scripts/other.cmd", b"inert")?;
    let root = runtime.record.root();
    let record = RuntimeRecord::new(
        runtime.record.id().clone(),
        root.into(),
        vec![root.into(), root.join("Scripts"), root.join("SCRIPTS")],
        runtime.record.origin().clone(),
    )?;
    let inventory = discover_commands(&record)?;
    assert_eq!(inventory.directories().len(), 2);
    assert_eq!(
        inventory
            .diagnostics()
            .iter()
            .filter(|d| d.reason == DiscoveryIssue::DuplicateDirectory)
            .count(),
        2
    );
    assert!(
        inventory
            .commands()
            .values()
            .all(|entry| entry.shadowed.is_empty())
    );
    Ok(())
}

#[test]
fn junction_source_is_rejected_and_command_junction_is_never_followed() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    let other = FakeRuntime::new("3.12.10", 2)?;
    other.write("hidden.cmd", b"inert")?;
    let source = runtime.record.root().join("Linked");
    junction::create(other.record.root(), &source)?;
    let record = RuntimeRecord::new(
        runtime.record.id().clone(),
        runtime.record.root().into(),
        vec![source],
        runtime.record.origin().clone(),
    )?;
    assert!(discover_commands(&record).is_err());
    junction::create(
        other.record.root(),
        runtime.record.root().join("escape.exe"),
    )?;
    let inventory = discover_commands(&runtime.record)?;
    assert_eq!(
        inventory.commands()[&bare("escape")?].winner.status,
        CommandStatus::Rejected(CommandRejection::ReparsePoint)
    );
    assert!(inventory.lookup(&bare("hidden")?).is_err());
    assert!(other.record.root().join("hidden.cmd").exists());
    Ok(())
}

#[test]
fn locked_file_aborts_instead_of_publishing_an_incomplete_inventory() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    let path = runtime.write("tool.exe", pe_header(3))?;
    let handle = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&path)?;
    assert_eq!(
        discover_commands(&runtime.record).err().map(|e| e.kind()),
        Some(ErrorKind::Busy)
    );
    drop(handle);
    assert!(
        discover_commands(&runtime.record)?
            .lookup(&bare("tool")?)
            .is_ok()
    );
    Ok(())
}

#[test]
fn entry_limits_include_unsupported_children_and_no_partial_result_is_returned() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    runtime.write("ignored.txt", b"inert")?;
    runtime.write("tool.cmd", b"inert")?;
    assert!(
        discover_commands_with_limits(
            &runtime.record,
            DiscoveryLimits {
                directories: 2,
                entries: 3
            }
        )
        .is_ok()
    );
    for limits in [
        DiscoveryLimits {
            directories: 2,
            entries: 2,
        },
        DiscoveryLimits {
            directories: 1,
            entries: 4,
        },
        DiscoveryLimits {
            directories: 65,
            entries: 4,
        },
        DiscoveryLimits {
            directories: 2,
            entries: 0,
        },
        DiscoveryLimits {
            directories: 2,
            entries: 16_385,
        },
    ] {
        assert_eq!(
            discover_commands_with_limits(&runtime.record, limits)
                .err()
                .map(|e| e.kind()),
            Some(ErrorKind::Usage)
        );
    }
    Ok(())
}

#[test]
fn scanning_never_executes_script_contents() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    let sentinel = runtime.record.root().join("MUST_NOT_EXIST");
    runtime.write(
        "tripwire.cmd",
        format!("@echo executed>\"{}\"\r\n", sentinel.display()),
    )?;
    assert!(
        discover_commands(&runtime.record)?
            .lookup(&bare("tripwire")?)
            .is_ok()
    );
    assert!(!sentinel.exists());
    Ok(())
}

#[test]
fn actual_compiled_console_image_is_classified_without_running_it() -> TestResult {
    let runtime = FakeRuntime::new("3.13.7", 1)?;
    fs::copy(
        std::env::current_exe()?,
        runtime.record.root().join("actual.exe"),
    )?;
    assert_eq!(
        discover_commands(&runtime.record)?
            .lookup(&bare("actual")?)?
            .status,
        CommandStatus::Available(CommandKind::PeConsole)
    );
    Ok(())
}
