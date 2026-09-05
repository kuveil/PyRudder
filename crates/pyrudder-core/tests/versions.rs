//! Interpreter version parsing and ordering contracts.
//! 解释器版本解析与排序契约。

use pyrudder_core::{
    ErrorKind, Result,
    version::{MAX_VERSION_LENGTH, PythonVersion, ReleaseLevel},
};

#[test]
fn stable_version_exposes_numeric_components() -> Result<()> {
    let version: PythonVersion = "3.13.7".parse()?;
    assert_eq!(version.major(), 3);
    assert_eq!(version.minor(), 13);
    assert_eq!(version.patch(), 7);
    assert_eq!(version.release(), ReleaseLevel::Final);
    assert!(version.is_stable());
    assert_eq!(version.to_string(), "3.13.7");
    Ok(())
}

#[test]
fn prerelease_suffixes_normalize_case() -> Result<()> {
    for (input, canonical, release) in [
        ("3.14.0A1", "3.14.0a1", ReleaseLevel::Alpha(1)),
        ("3.14.0B2", "3.14.0b2", ReleaseLevel::Beta(2)),
        ("3.14.0RC10", "3.14.0rc10", ReleaseLevel::Candidate(10)),
    ] {
        let version: PythonVersion = input.parse()?;
        assert_eq!(version.to_string(), canonical);
        assert_eq!(version.release(), release);
        assert!(!version.is_stable());
    }
    Ok(())
}

#[test]
fn numeric_versions_and_prerelease_serials_sort_correctly() -> Result<()> {
    let ordered = [
        "3.9.9",
        "3.9.10",
        "3.13.7",
        "3.14.0a9",
        "3.14.0a10",
        "3.14.0b0",
        "3.14.0rc0",
        "3.14.0rc9",
        "3.14.0rc10",
        "3.14.0",
        "3.14.1a0",
        "4.0.0a0",
    ];
    let versions = ordered
        .iter()
        .map(|input| input.parse::<PythonVersion>())
        .collect::<Result<Vec<_>>>()?;
    assert!(versions.windows(2).all(|pair| pair[0] < pair[1]));
    Ok(())
}

#[test]
fn malformed_or_package_only_versions_are_rejected() {
    for input in [
        "",
        "3",
        "3.13",
        "3.13.",
        ".13.7",
        "3..7",
        "3.13.7.0",
        "03.13.7",
        "3.013.7",
        "3.13.07",
        "+3.13.7",
        "-3.13.7",
        "3.13.-7",
        "3.13.7 ",
        " 3.13.7",
        "3.13.7\n",
        "3.13.7\0",
        "３.13.7",
        "3.13.七",
        "3.14.0rc",
        "3.14.0rc01",
        "3.14.0c1",
        "3.14.0alpha1",
        "3.14.0a-1",
        "3.14.0-rc1",
        "3.14.0a1b1",
        "3.13.7.dev1",
        "3.13.7.post1",
        "3.13.7+local",
        "1!3.13.7",
    ] {
        assert!(
            matches!(input.parse::<PythonVersion>(), Err(error) if error.kind() == ErrorKind::Usage),
            "unexpected acceptance: {input:?}"
        );
    }
}

#[test]
fn component_overflow_and_excessive_length_are_rejected() {
    for input in [
        "4294967296.1.0",
        "3.4294967296.0",
        "3.13.4294967296",
        "3.14.0rc4294967296",
        "99999999999999999999.0.0",
    ] {
        assert!(input.parse::<PythonVersion>().is_err(), "{input}");
    }
    assert!(
        "3".repeat(MAX_VERSION_LENGTH + 1)
            .parse::<PythonVersion>()
            .is_err()
    );
}

#[test]
fn numeric_boundary_values_round_trip() -> Result<()> {
    for major in [0, 1, 3, 10, u32::MAX] {
        for minor in [0, 9, 13, u32::MAX] {
            for patch in [0, 7, 10, u32::MAX] {
                for release in [
                    ReleaseLevel::Alpha(0),
                    ReleaseLevel::Alpha(u32::MAX),
                    ReleaseLevel::Beta(10),
                    ReleaseLevel::Candidate(u32::MAX),
                    ReleaseLevel::Final,
                ] {
                    let version = PythonVersion::new(major, minor, patch).with_release(release);
                    assert_eq!(version.to_string().parse::<PythonVersion>()?, version);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn malformed_versions_never_echo_terminal_control_sequences() {
    let result = "3.13.7\u{1b}[31mprivate".parse::<PythonVersion>();
    assert!(matches!(result, Err(error) if
        !error.to_string().contains('\u{1b}') && !error.to_string().contains("private")));
}
