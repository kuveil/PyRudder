//! Public CLI output and exit-code regression tests.
//! 公开 CLI 输出与退出码回归测试。

use std::process::Command;

#[test]
fn available_exposes_explicit_noninteractive_mode() -> std::io::Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_pyrudder"))
        .args(["available", "--help"])
        .output()?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8_lossy(&output.stdout);
    for option in ["--list", "--offline", "--json", "--quiet"] {
        assert!(help.contains(option), "{option}");
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn installed_package_defaults_stay_in_its_own_root() -> Result<(), Box<dyn std::error::Error>> {
    use pyrudder_core::state::StateFileSystem;
    use pyrudder_platform_windows::{registry::Location, state::WindowsStateFileSystem};
    use std::fs;

    let temporary = tempfile::tempdir()?;
    let root = WindowsStateFileSystem.normalize_directory(temporary.path())?;
    let location = Location {
        schema_version: 1,
        config_dir: root.join("config"),
        install_dir: root.join("bin"),
        shims_dir: root.join("shims"),
    };
    for directory in [
        &location.config_dir,
        &location.install_dir,
        &location.shims_dir,
    ] {
        fs::create_dir(directory)?;
        location.publish(directory)?;
    }
    let executable = location.install_dir.join("pyrudder.exe");
    fs::copy(env!("CARGO_BIN_EXE_pyrudder"), &executable)?;
    fs::write(
        location.install_dir.join("pyrudder-layout.json"),
        br#"{"schema_version":1,"layout":"in-place"}"#,
    )?;
    // Missing path settings model config unset without mutating the host's real installation.
    // 缺失路径设置模拟 config unset，不修改宿主机器上的真实安装。
    fs::write(
        location.config_dir.join("config.toml"),
        "schema_version = 1\n",
    )?;
    let mut command = Command::new(&executable);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PYRUDDER_") {
            command.env_remove(name);
        }
    }
    let output = command.args(["config", "get", "--json"]).output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    for name in ["runtimes", "downloads", "cache", "temp"] {
        assert_eq!(
            response["data"]["effective_paths"][format!("{name}_dir")],
            root.join(name).to_string_lossy().as_ref()
        );
    }
    Ok(())
}

#[test]
fn invalid_arguments_use_the_shared_usage_exit_code() -> std::io::Result<()> {
    for arguments in [
        vec!["--unknown"],
        vec!["global", "3.13", "extra"],
        vec!["--version", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_pyrudder"))
            .args(arguments)
            .output()?;
        assert_eq!(
            output.status.code(),
            Some(i32::from(pyrudder_core::ErrorKind::Usage.exit_code()))
        );
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage: pyrudder"));
    }
    Ok(())
}

#[test]
fn version_output_has_no_banner_or_diagnostics() -> std::io::Result<()> {
    for flag in ["-V", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_pyrudder"))
            .arg(flag)
            .output()?;
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            concat!("pyrudder ", env!("CARGO_PKG_VERSION"))
        );
    }
    Ok(())
}
