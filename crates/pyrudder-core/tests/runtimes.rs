//! Runtime identity, registration, and in-memory selection contracts.
//! 运行时标识、登记记录与内存选择契约。

use std::path::PathBuf;

use pyrudder_core::{
    ErrorKind, Result,
    runtime::{
        Architecture, Implementation, InstallationId, MAX_ALIAS_LENGTH, MAX_RUNTIME_ID_LENGTH,
        RuntimeAlias, RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord, RuntimeVariant,
    },
    selector::{RuntimeSelection, VersionSelector},
    version::{PythonVersion, ReleaseLevel},
};

// Synthetic native absolute paths; tests never touch or execute these locations.
// 合成的平台原生绝对路径；测试不会访问这些位置，也不会执行其中的文件。
fn root() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\pyrudder-model-tests\团队 Python")
    } else {
        PathBuf::from("/pyrudder-model-tests/团队 Python")
    }
}

fn managed_origin() -> Result<RuntimeOrigin> {
    Ok(RuntimeOrigin::Managed {
        provider: "pythonorg".to_owned(),
        artifact_url: "https://example.invalid/python.zip".to_owned(),
        artifact_sha256: [0xab; 32],
        ownership_id: InstallationId::new(1)?,
    })
}

fn ready(version: &str, installation: Option<u128>) -> Result<RuntimeRecord> {
    let mut id = RuntimeId::new(version.parse()?)?;
    let origin = if let Some(value) = installation {
        id = id.with_installation(InstallationId::new(value)?);
        RuntimeOrigin::External {
            registered_executable: root().join(format!("instance-{value}/python.exe")),
        }
    } else {
        managed_origin()?
    };
    let mut record = RuntimeRecord::new(id, root(), vec![root().join("Scripts")], origin)?;
    record.set_health(RuntimeHealth::Ready);
    Ok(record)
}

fn assert_error<T>(result: Result<T>, kind: ErrorKind) {
    assert!(matches!(result, Err(error) if error.kind() == kind));
}

#[test]
fn runtime_id_normalizes_and_exposes_supported_dimensions() -> Result<()> {
    let id: RuntimeId = "CPYTHON-3.13.7-X64".parse()?;
    assert_eq!(id.to_string(), "cpython-3.13.7-x64");
    assert_eq!(id.implementation(), Implementation::Cpython);
    assert_eq!(id.version(), PythonVersion::new(3, 13, 7));
    assert_eq!(id.architecture(), Architecture::X64);
    assert_eq!(id.variant(), RuntimeVariant::Standard);
    assert_eq!(id.installation(), None);
    Ok(())
}

#[test]
fn installation_qualifiers_normalize_and_distinguish_same_version() -> Result<()> {
    let qualified: RuntimeId = "CPYTHON-3.13.7-X64@ABCDEF0123456789ABCDEF0123456789".parse()?;
    assert_eq!(
        qualified.to_string(),
        "cpython-3.13.7-x64@abcdef0123456789abcdef0123456789"
    );
    assert_eq!(qualified.to_string().parse::<RuntimeId>()?, qualified);
    let canonical: RuntimeId = "cpython-3.13.7-x64".parse()?;
    assert_ne!(canonical, qualified);
    assert_ne!(
        qualified,
        canonical.with_installation(InstallationId::new(1)?)
    );
    for value in [1, 10, u128::MAX] {
        let installation = InstallationId::new(value)?;
        assert_eq!(installation.value(), value);
        assert_eq!(
            installation.to_string().parse::<InstallationId>()?,
            installation
        );
    }
    Ok(())
}

#[test]
fn invalid_installation_identifiers_are_rejected() {
    assert_error(InstallationId::new(0), ErrorKind::Usage);
    for input in [
        "",
        "1",
        "00000000000000000000000000000000",
        "0000000000000000000000000000000g",
        "00000000-0000-0000-0000-000000000001",
        "000000000000000000000000000000001",
        "0000000000000000000000000000001",
        "00000000000000000000000000000001 ",
    ] {
        assert_error(input.parse::<InstallationId>(), ErrorKind::Usage);
    }
}

#[test]
fn runtime_ids_reject_unsupported_builds_and_invalid_syntax() {
    for input in [
        "",
        "cpython-3.13-x64",
        "cpython-03.13.7-x64",
        "cpython-3.13.7",
        "pypy-3.13.7-x64",
        "cpython-3.13.7-x86",
        "cpython-3.13.7-arm64",
        "cpython-3.13.7-x64-debug",
        "cpython-3.13.7t-x64",
        "cpython-3.14.0rc1-x64",
        "cpython-3.13.7-x64@",
        "cpython-3.13.7-x64@1",
        "cpython-3.13.7-x64@00000000000000000000000000000001@00000000000000000000000000000002",
        "cpython-3.13.7-x64\n",
        "cpython-3.13.七-x64",
    ] {
        assert_error(input.parse::<RuntimeId>(), ErrorKind::Usage);
    }
    assert_error(
        "a".repeat(MAX_RUNTIME_ID_LENGTH + 1).parse::<RuntimeId>(),
        ErrorKind::Usage,
    );
    let prerelease = PythonVersion::new(3, 14, 0).with_release(ReleaseLevel::Candidate(1));
    assert_error(RuntimeId::new(prerelease), ErrorKind::Usage);
}

#[test]
fn aliases_are_case_insensitive_and_length_bounded() -> Result<()> {
    let alias: RuntimeAlias = "Work_Python-3".parse()?;
    assert_eq!(alias.as_str(), "work_python-3");
    assert_eq!(alias, "WORK_PYTHON-3".parse()?);
    assert_eq!(alias.to_string().parse::<RuntimeAlias>()?, alias);
    assert!("a".repeat(MAX_ALIAS_LENGTH).parse::<RuntimeAlias>().is_ok());
    assert_error(
        "a".repeat(MAX_ALIAS_LENGTH + 1).parse::<RuntimeAlias>(),
        ErrorKind::Usage,
    );
    Ok(())
}

#[test]
fn aliases_reject_reserved_names_paths_and_metacharacters() {
    for input in [
        "",
        "SYSTEM",
        "PyRudder",
        "cpython-broken",
        "CON",
        "prn",
        "aux",
        "nul",
        "com1",
        "COM9",
        "lpt1",
        "LPT9",
        "3work",
        "_work",
        "-work",
        "work.name",
        "工作",
        "work ",
        "work\n",
        "a/b",
        r"a\b",
        "a:b",
        "a@b",
        "a&b",
        "a|b",
        "a;b",
        "a\0",
        "a\u{1b}[31m",
    ] {
        assert_error(input.parse::<RuntimeAlias>(), ErrorKind::Usage);
    }
}

#[test]
fn all_selector_forms_have_canonical_round_trips() -> Result<()> {
    for (input, canonical) in [
        ("3.13", "3.13"),
        ("3.13.7", "3.13.7"),
        ("CPYTHON-3.13.7-X64", "cpython-3.13.7-x64"),
        (
            "cpython-3.13.7-x64@00000000000000000000000000000001",
            "cpython-3.13.7-x64@00000000000000000000000000000001",
        ),
        ("Work", "work"),
        ("SYSTEM", "system"),
    ] {
        let selector: VersionSelector = input.parse()?;
        assert_eq!(selector.to_string(), canonical);
        assert_eq!(selector.to_string().parse::<VersionSelector>()?, selector);
    }
    assert_eq!(
        "3.13".parse::<VersionSelector>()?,
        VersionSelector::Minor {
            major: 3,
            minor: 13
        }
    );
    assert_eq!(
        "3.13.7".parse::<VersionSelector>()?,
        VersionSelector::Exact(PythonVersion::new(3, 13, 7))
    );
    assert_eq!(
        "SYSTEM".parse::<VersionSelector>()?,
        VersionSelector::System
    );
    Ok(())
}

#[test]
fn selectors_reject_malformed_versions_and_ids_without_alias_fallback() {
    for input in [
        "",
        "3",
        "3.",
        "03.13",
        "3.013",
        "3.-1",
        "4294967296.13",
        "3.4294967296",
        "3.13rc1",
        "3.14.0rc1",
        "cpython-broken",
        "CPYTHON-3.13.7-ARM64",
        "3.13.7/evil",
        "3.13\n",
        "3.13.7\0",
        "C:\\Python",
        "工作",
        "con",
    ] {
        assert_error(input.parse::<VersionSelector>(), ErrorKind::Usage);
    }
    assert_error(
        "a".repeat(MAX_RUNTIME_ID_LENGTH + 1)
            .parse::<VersionSelector>(),
        ErrorKind::Usage,
    );
    let prerelease = PythonVersion::new(3, 14, 0).with_release(ReleaseLevel::Alpha(1));
    assert_error(
        VersionSelector::Exact(prerelease).select(&[]),
        ErrorKind::Usage,
    );
}

#[test]
fn ascii_mutations_never_panic_or_produce_non_round_trippable_selectors() -> Result<()> {
    for seed in ["3.13", "3.13.7", "cpython-3.13.7-x64", "work", "system"] {
        for position in 0..seed.len() {
            for byte in 0_u8..=127 {
                let mut input = seed.to_owned();
                input.replace_range(position..=position, &char::from(byte).to_string());
                if let Ok(selector) = input.parse::<VersionSelector>() {
                    assert_eq!(selector.to_string().parse::<VersionSelector>()?, selector);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn new_records_are_unchecked_and_aliases_do_not_change_identity() -> Result<()> {
    let id: RuntimeId = "cpython-3.13.7-x64".parse()?;
    let origin = managed_origin()?;
    // A distinct absolute scripts path is allowed; containment is not a domain-model claim.
    // 允许独立的绝对脚本路径；领域模型不承诺目录包含关系。
    let directories = vec![
        root().join("Scripts"),
        root().with_file_name("User Scripts"),
    ];
    let mut record = RuntimeRecord::new(id.clone(), root(), directories.clone(), origin.clone())?;
    assert_eq!(record.health(), &RuntimeHealth::Unchecked);
    assert!(!record.health().is_ready());
    assert_eq!(record.root(), root());
    assert_eq!(record.command_dirs(), directories);
    assert_eq!(record.origin(), &origin);
    assert!(record.add_alias("Work".parse()?));
    assert!(!record.add_alias("WORK".parse()?));
    assert_eq!(record.aliases().len(), 1);
    assert_eq!(record.id(), &id);
    Ok(())
}

#[test]
fn external_records_require_a_qualified_identity() -> Result<()> {
    let id: RuntimeId = "cpython-3.13.7-x64".parse()?;
    let executable = root().join("python.exe");
    let origin = RuntimeOrigin::External {
        registered_executable: executable.clone(),
    };
    assert_error(
        RuntimeRecord::new(
            id.clone(),
            root(),
            vec![root().join("Scripts")],
            origin.clone(),
        ),
        ErrorKind::Usage,
    );
    let record = RuntimeRecord::new(
        id.with_installation(InstallationId::new(7)?),
        root(),
        vec![root().join("Scripts")],
        origin,
    )?;
    assert_eq!(
        record.origin(),
        &RuntimeOrigin::External {
            registered_executable: executable
        }
    );
    assert_eq!(record.health(), &RuntimeHealth::Unchecked);
    Ok(())
}

#[test]
fn records_reject_relative_parent_traversing_and_missing_script_paths() -> Result<()> {
    let id: RuntimeId = "cpython-3.13.7-x64".parse()?;
    for path in [
        PathBuf::new(),
        PathBuf::from("relative"),
        root().join("../other"),
    ] {
        assert_error(
            RuntimeRecord::new(id.clone(), path.clone(), vec![root()], managed_origin()?),
            ErrorKind::Usage,
        );
        assert_error(
            RuntimeRecord::new(id.clone(), root(), vec![path.clone()], managed_origin()?),
            ErrorKind::Usage,
        );
        assert_error(
            RuntimeRecord::new(
                id.clone().with_installation(InstallationId::new(1)?),
                root(),
                vec![root()],
                RuntimeOrigin::External {
                    registered_executable: path,
                },
            ),
            ErrorKind::Usage,
        );
    }
    assert_error(
        RuntimeRecord::new(id, root(), vec![], managed_origin()?),
        ErrorKind::Usage,
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_drive_relative_and_root_relative_paths_are_rejected() -> Result<()> {
    for path in [r"C:Python", r"\Python", r"\", r"C:\Python\..\Other"] {
        assert_error(
            RuntimeRecord::new(
                "cpython-3.13.7-x64".parse()?,
                PathBuf::from(path),
                vec![root()],
                managed_origin()?,
            ),
            ErrorKind::Usage,
        );
    }
    Ok(())
}

#[test]
fn minor_selection_uses_numeric_latest_patch_and_ignores_other_series() -> Result<()> {
    let records = [
        ready("3.13.9", None)?,
        ready("3.13.10", None)?,
        ready("3.14.0", None)?,
        ready("4.13.99", None)?,
    ];
    let selector: VersionSelector = "3.13".parse()?;
    assert_eq!(
        selector.select(&records)?,
        RuntimeSelection::Registered(&records[1])
    );
    let reversed: Vec<_> = records.iter().rev().cloned().collect();
    assert_eq!(
        selector.select(&reversed)?,
        RuntimeSelection::Registered(&records[1])
    );
    Ok(())
}

#[test]
fn exact_versions_and_aliases_do_not_drift_to_newer_versions() -> Result<()> {
    let mut pinned = ready("3.13.7", None)?;
    pinned.add_alias("Work".parse()?);
    let records = [pinned, ready("3.13.10", None)?];
    for input in ["3.13.7", "WORK", "cpython-3.13.7-x64"] {
        assert_eq!(
            input.parse::<VersionSelector>()?.select(&records)?,
            RuntimeSelection::Registered(&records[0])
        );
    }
    Ok(())
}

#[test]
fn multiple_installations_require_an_exact_id_or_unique_alias() -> Result<()> {
    let mut second = ready("3.13.7", Some(2))?;
    second.add_alias("Work".parse()?);
    let records = [ready("3.13.7", Some(1))?, second];
    for input in ["3.13", "3.13.7"] {
        assert_error(
            input.parse::<VersionSelector>()?.select(&records),
            ErrorKind::Conflict,
        );
    }
    for input in [records[1].id().to_string(), "work".to_owned()] {
        assert_eq!(
            input.parse::<VersionSelector>()?.select(&records)?,
            RuntimeSelection::Registered(&records[1])
        );
    }
    assert_error(
        "cpython-3.13.7-x64"
            .parse::<VersionSelector>()?
            .select(&records),
        ErrorKind::NotInstalled,
    );
    Ok(())
}

#[test]
fn managed_and_external_copies_are_ambiguous_for_version_selectors() -> Result<()> {
    let records = [ready("3.13.7", None)?, ready("3.13.7", Some(1))?];
    assert_error(
        "3.13.7".parse::<VersionSelector>()?.select(&records),
        ErrorKind::Conflict,
    );
    assert_eq!(
        "cpython-3.13.7-x64"
            .parse::<VersionSelector>()?
            .select(&records)?,
        RuntimeSelection::Registered(&records[0])
    );
    Ok(())
}

#[test]
fn all_nonready_states_fail_without_selecting_an_older_patch() -> Result<()> {
    for health in [
        RuntimeHealth::Unchecked,
        RuntimeHealth::Broken {
            reason: "missing python.exe".to_owned(),
        },
        RuntimeHealth::Unavailable {
            reason: "drive disconnected".to_owned(),
        },
        RuntimeHealth::PendingRemoval,
    ] {
        let mut latest = ready("3.13.10", None)?;
        latest.set_health(health);
        let mut records = [ready("3.13.7", None)?, latest];
        for input in ["3.13", "3.13.10", "cpython-3.13.10-x64"] {
            assert_error(
                input.parse::<VersionSelector>()?.select(&records),
                ErrorKind::BrokenRuntime,
            );
        }
        records[1].set_health(RuntimeHealth::Ready);
        assert_eq!(
            "3.13".parse::<VersionSelector>()?.select(&records)?,
            RuntimeSelection::Registered(&records[1])
        );
    }
    Ok(())
}

#[test]
fn ambiguous_latest_patch_never_falls_back_to_older_or_healthier_copy() -> Result<()> {
    let mut broken = ready("3.13.10", Some(2))?;
    broken.set_health(RuntimeHealth::Broken {
        reason: "damaged".to_owned(),
    });
    let records = [ready("3.13.7", None)?, ready("3.13.10", Some(1))?, broken];
    assert_error(
        "3.13".parse::<VersionSelector>()?.select(&records),
        ErrorKind::Conflict,
    );
    Ok(())
}

#[test]
fn conflicts_in_older_patches_do_not_block_unique_latest_patch() -> Result<()> {
    let records = [
        ready("3.13.7", Some(1))?,
        ready("3.13.7", Some(2))?,
        ready("3.13.10", None)?,
    ];
    assert_eq!(
        "3.13".parse::<VersionSelector>()?.select(&records)?,
        RuntimeSelection::Registered(&records[2])
    );
    Ok(())
}

#[test]
fn duplicate_aliases_and_duplicate_ids_are_conflicts() -> Result<()> {
    let mut first = ready("3.13.7", None)?;
    let mut second = ready("3.14.0", None)?;
    first.add_alias("work".parse()?);
    second.add_alias("WORK".parse()?);
    assert_error(
        "work"
            .parse::<VersionSelector>()?
            .select(&[first.clone(), second]),
        ErrorKind::Conflict,
    );
    assert_error(
        "cpython-3.13.7-x64"
            .parse::<VersionSelector>()?
            .select(&[first.clone(), first]),
        ErrorKind::Conflict,
    );
    Ok(())
}

#[test]
fn conflict_diagnostics_are_independent_of_registration_order() -> Result<()> {
    let records = [ready("3.13.7", Some(2))?, ready("3.13.7", Some(1))?];
    let selector: VersionSelector = "3.13.7".parse()?;
    let first = selector.select(&records).err();
    let reversed = [records[1].clone(), records[0].clone()];
    let second = selector.select(&reversed).err();
    assert!(first.is_some());
    assert_eq!(first, second);
    assert!(
        matches!(first, Some(error) if error.message().contains(&records[0].id().to_string())
        && error.message().contains(&records[1].id().to_string()) && error.hint().is_some())
    );
    Ok(())
}

#[test]
fn missing_selections_do_not_implicitly_request_system_fallback() -> Result<()> {
    let records = [ready("3.13.7", None)?];
    for input in [
        "3.12",
        "3.13.8",
        "absent",
        "cpython-3.13.7-x64@00000000000000000000000000000001",
    ] {
        assert_error(
            input.parse::<VersionSelector>()?.select(&records),
            ErrorKind::NotInstalled,
        );
    }
    assert_error(
        "3.13".parse::<VersionSelector>()?.select(&[]),
        ErrorKind::NotInstalled,
    );
    assert_eq!(
        VersionSelector::System.select(&records)?,
        RuntimeSelection::SystemRequested
    );
    assert_eq!(
        VersionSelector::System.select(&[])?,
        RuntimeSelection::SystemRequested
    );
    Ok(())
}
