//! Pure encoding specifications; these tests never start interpreters or create files.
//! 纯编码规则断言；这些测试不启动解释器或创建文件。

use super::{CMD_MAX_UNITS, cmd_command, powershell_command};
use pyrudder_core::{ErrorKind, Result};
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[test]
fn cmd_quotes_literals_without_crt_backslash_escaping() -> Result<()> {
    let arguments = ["", "a b", "中文", r"D:\tail\", "a&b|c<d>e^(f)"].map(OsString::from);
    assert_eq!(
        cmd_command(
            Path::new(r"C:\Windows\System32\cmd.exe"),
            Path::new(r"D:\My (Scripts)\check.cmd"),
            &arguments
        )?,
        units(
            r#""C:\Windows\System32\cmd.exe" /d /s /v:off /e:on /c ""D:\My (Scripts)\check.cmd" "" "a b" "中文" "D:\tail\" "a&b|c<d>e^(f)"""#
        )
    );
    Ok(())
}

#[test]
fn cmd_rejects_expansion_quotes_and_controls_in_paths_and_arguments() {
    for value in [
        "%PATH%",
        "!name!",
        "a\"&whoami",
        "a\rb",
        "a\nb",
        "a\tb",
        "a\0b",
        "a\u{85}b",
    ] {
        let argument = cmd_command(
            Path::new(r"C:\cmd.exe"),
            Path::new(r"D:\check.cmd"),
            &[value.into()],
        );
        assert!(matches!(argument, Err(error) if error.kind() == ErrorKind::Usage));
        let path = format!(r"D:\{value}\check.bat");
        let script = cmd_command(Path::new(r"C:\cmd.exe"), Path::new(&path), &[]);
        assert!(matches!(script, Err(error) if error.kind() == ErrorKind::Usage));
    }
}

#[test]
fn cmd_enforces_the_encoded_limit_including_quotes() -> Result<()> {
    let image = Path::new(r"C:\cmd.exe");
    let script = Path::new(r"D:\check.bat");
    let baseline = cmd_command(image, script, &[OsString::new()])?.len() - 1;
    let argument = "x".repeat(CMD_MAX_UNITS - baseline);
    assert_eq!(
        cmd_command(image, script, &[argument.clone().into()])?.len(),
        CMD_MAX_UNITS + 1
    );
    assert!(
        matches!(cmd_command(image, script, &[format!("{argument}x").into()]), Err(error) if error.kind() == ErrorKind::Usage)
    );
    Ok(())
}

#[test]
fn powershell_uses_file_and_literal_host_arguments() -> Result<()> {
    let arguments = [
        "",
        r#"a"b"#,
        r"D:\tail\",
        "$x;&|%PATH%!",
        "-ExecutionPolicy",
        "Bypass",
    ]
    .map(OsString::from);
    assert_eq!(
        powershell_command(
            Path::new(r"C:\powershell.exe"),
            Path::new(r"D:\My Scripts\check.ps1"),
            &arguments
        )?,
        units(
            r#""C:\powershell.exe" "-NoLogo" "-NoProfile" "-File" "D:\My Scripts\check.ps1" "" "a\"b" "D:\tail\\" "$x;&|%PATH%!" "-ExecutionPolicy" "Bypass""#
        )
    );
    Ok(())
}

#[test]
fn script_adapters_reject_nul_and_unpaired_surrogates() {
    for value in [OsString::from("a\0b"), OsString::from_wide(&[0xd800])] {
        let arguments = [value];
        assert!(
            matches!(cmd_command(Path::new(r"C:\cmd.exe"), Path::new(r"D:\check.cmd"), &arguments), Err(error) if error.kind() == ErrorKind::Usage)
        );
        assert!(
            matches!(powershell_command(Path::new(r"C:\powershell.exe"), Path::new(r"D:\check.ps1"), &arguments), Err(error) if error.kind() == ErrorKind::Usage)
        );
    }
}
