//! Child-only environment construction; no process-global environment mutation.
//! 仅构造子进程环境；不修改进程全局环境。

use super::DispatchContext;
use crate::{
    casing,
    state::{WindowsStateFileSystem, normalize_path},
};
use pyrudder_core::{Error, ErrorKind, Result, runtime::RuntimeRecord, state::StateFileSystem};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf, Prefix},
};

const MAX_ENVIRONMENT_UNITS: usize = 1_048_576;

pub(super) fn dos_path(path: &Path) -> PathBuf {
    if matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)))
    {
        let units: Vec<u16> = path.as_os_str().encode_wide().skip(4).collect();
        PathBuf::from(OsString::from_wide(&units))
    } else {
        path.to_path_buf()
    }
}

pub(super) fn path_key(path: &Path) -> Result<PathBuf> {
    let path = dos_path(path);
    let path = path.to_str().ok_or_else(|| {
        Error::new(
            ErrorKind::Usage,
            "Process search directories must be Unicode",
        )
    })?;
    Ok(PathBuf::from(casing::uppercase(path)?))
}

pub(super) fn build(
    context: &DispatchContext<'_>,
    active: &RuntimeRecord,
) -> Result<(Vec<u16>, Vec<PathBuf>)> {
    let mut variables = BTreeMap::new();
    for (name, value) in context.environment {
        insert(&mut variables, name, value)?;
    }
    let inherited_path = variables
        .get(&"PATH".encode_utf16().collect::<Vec<_>>())
        .map(|(_, value)| value.clone());
    let (path, filtered) = child_path(context, active, inherited_path.as_deref())?;
    for (name, value) in [
        ("PATH", path),
        ("PYRUDDER_VERSION", OsString::from(active.id().to_string())),
        (
            "PYRUDDER_ACTIVE_RUNTIME",
            OsString::from(active.id().to_string()),
        ),
        (
            "PYRUDDER_ACTIVE_ROOT",
            dos_path(active.root()).into_os_string(),
        ),
    ] {
        insert(&mut variables, OsStr::new(name), &value)?;
    }
    Ok((encode(&variables)?, filtered))
}

fn encode(variables: &BTreeMap<Vec<u16>, (OsString, OsString)>) -> Result<Vec<u16>> {
    let mut block = Vec::new();
    for (name, value) in variables.values() {
        for unit in name
            .encode_wide()
            .chain(std::iter::once(u16::from(b'=')))
            .chain(value.encode_wide())
            .chain(std::iter::once(0))
        {
            if block.len() >= MAX_ENVIRONMENT_UNITS - 1 {
                return Err(Error::new(
                    ErrorKind::Usage,
                    "Child environment exceeds its size limit",
                ));
            }
            block.push(unit);
        }
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn insert(
    variables: &mut BTreeMap<Vec<u16>, (OsString, OsString)>,
    name: &OsStr,
    value: &OsStr,
) -> Result<()> {
    let units: Vec<_> = name.encode_wide().collect();
    // CMD inherits hidden entries such as =ExitCode as well as drive directories (=C:).
    // Permit the leading marker, but never a second delimiter or an empty hidden name.
    // CMD 会继承 =ExitCode 等隐藏项，以及盘符工作目录（=C:）。
    // 允许前导标记，但拒绝第二个分隔符或空隐藏名称。
    let visible_name = units.strip_prefix(&[u16::from(b'=')]).unwrap_or(&units);
    if visible_name.is_empty()
        || units.contains(&0)
        || visible_name.contains(&u16::from(b'='))
        || value.encode_wide().any(|unit| unit == 0)
    {
        return Err(Error::new(
            ErrorKind::Usage,
            "Invalid child environment entry",
        ));
    }
    let key = casing::uppercase_wide(&units)?;
    variables.insert(key, (name.to_os_string(), value.to_os_string()));
    Ok(())
}

fn child_path(
    context: &DispatchContext<'_>,
    active: &RuntimeRecord,
    inherited: Option<&OsStr>,
) -> Result<(OsString, Vec<PathBuf>)> {
    let mut blocked = Vec::new();
    for runtime in context.runtimes {
        for directory in std::iter::once(runtime.root())
            .chain(runtime.command_dirs().iter().map(PathBuf::as_path))
        {
            if let Ok(lexical) = normalize_path(directory).and_then(|path| path_key(&path)) {
                blocked.push(lexical);
            }
            // Other unavailable registrations must not stop an explicitly selected healthy runtime.
            // 其他不可用登记不能阻止显式选中的健康运行时。
            if let Ok(canonical) = WindowsStateFileSystem
                .normalize_directory(directory)
                .and_then(|path| path_key(&path))
            {
                blocked.push(canonical);
            }
        }
    }
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for directory in std::iter::once(active.root())
        .chain(active.command_dirs().iter().map(PathBuf::as_path))
        .chain(context.shims_dir)
    {
        let directory = dos_path(&WindowsStateFileSystem.normalize_directory(directory)?);
        if seen.insert(path_key(&directory)?) {
            paths.push(directory);
        }
    }
    if let Some(shims) = context.shims_dir {
        blocked.push(path_key(
            &WindowsStateFileSystem.normalize_directory(shims)?,
        )?);
    }
    let mut filtered = Vec::new();
    if let Some(inherited) = inherited {
        for directory in std::env::split_paths(inherited) {
            let canonical = WindowsStateFileSystem.normalize_directory(&directory);
            if let Ok(canonical) = canonical {
                let canonical = dos_path(&canonical);
                if let Ok(key) = path_key(&canonical) {
                    if !blocked.iter().any(|blocked| key.starts_with(blocked)) && seen.insert(key) {
                        paths.push(canonical);
                        continue;
                    }
                }
            }
            filtered.push(directory);
        }
    }
    let joined = std::env::join_paths(paths)
        .map_err(|_| Error::new(ErrorKind::Usage, "Cannot encode the child PATH"))?;
    Ok((joined, filtered))
}

#[cfg(test)]
mod tests {
    use super::{encode, insert};
    use std::{
        collections::BTreeMap,
        error::Error,
        ffi::{OsStr, OsString},
        os::windows::{ffi::OsStringExt, process::CommandExt},
        process::Command,
    };

    #[test]
    fn accepts_real_cmd_environment_after_a_child_exit() -> Result<(), Box<dyn Error>> {
        const CHILD_FLAG: &str = "PYRUDDER_TEST_CMD_ENV_CHILD";
        if std::env::var_os(CHILD_FLAG).is_some() {
            let environment: Vec<_> = std::env::vars_os().collect();
            assert!(environment.iter().any(|(name, _)| name == "=ExitCode"));
            let mut variables = BTreeMap::new();
            for (name, value) in &environment {
                if name.to_string_lossy().starts_with('=') {
                    println!("Inherited CMD hidden variable: {name:?}");
                }
                insert(&mut variables, name, value)?;
            }
            return Ok(());
        }
        let image = std::env::current_exe()?;
        let cmd = std::path::PathBuf::from(std::env::var("SystemRoot")?)
            .join("System32")
            .join("cmd.exe");
        let output = Command::new(cmd)
            .args(["/d", "/c"])
            .raw_arg(format!(
                "cmd /d /c exit 37 & \"{}\" --exact process::environment::tests::accepts_real_cmd_environment_after_a_child_exit --nocapture",
                image.display()
            ))
            .env(CHILD_FLAG, "1")
            .output()?;
        assert!(
            output.status.success(),
            "CMD child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
        Ok(())
    }

    #[test]
    fn preserves_hidden_and_non_unicode_entries_through_native_launch() -> Result<(), Box<dyn Error>>
    {
        const CHILD_FLAG: &str = "PYRUDDER_TEST_NATIVE_ENV_CHILD";
        let native_name = OsString::from_wide(&[0xd800, u16::from(b'p')]);
        let native_uppercase = OsString::from_wide(&[0xd800, u16::from(b'P')]);
        let native_value = OsString::from_wide(&[u16::from(b'v'), 0xdc00]);
        if std::env::var_os(CHILD_FLAG).is_some() {
            let environment: Vec<_> = std::env::vars_os().collect();
            assert!(
                environment
                    .iter()
                    .any(|(name, value)| { name == &native_uppercase && value == &native_value })
            );
            assert!(!environment.iter().any(|(name, _)| name == &native_name));
            assert!(
                environment
                    .iter()
                    .any(|(name, value)| { name == "=ExitCode" && value == "00000025" })
            );
            assert!(
                environment
                    .iter()
                    .any(|(name, value)| { name == "=ExitCodeAscii" && value == "%" })
            );
            assert!(
                environment
                    .iter()
                    .any(|(name, value)| { name == "=C:" && value == r"C:\Windows" })
            );
            return Ok(());
        }

        let mut variables = BTreeMap::new();
        for (name, value) in std::env::vars_os() {
            insert(&mut variables, &name, &value)?;
        }
        insert(&mut variables, &native_name, OsStr::new("replaced"))?;
        insert(&mut variables, &native_uppercase, &native_value)?;
        for (name, value) in [
            (CHILD_FLAG, "1"),
            ("=ExitCode", "00000025"),
            ("=ExitCodeAscii", "%"),
            ("=C:", r"C:\Windows"),
        ] {
            insert(&mut variables, OsStr::new(name), OsStr::new(value))?;
        }
        let image = std::env::current_exe()?;
        let arguments = [
            OsString::from("--exact"),
            OsString::from(
                "process::environment::tests::preserves_hidden_and_non_unicode_entries_through_native_launch",
            ),
            OsString::from("--nocapture"),
        ];
        let mut command = crate::native_process::command_line(&image, &arguments)?;
        let mut environment = encode(&variables)?;
        let exit = crate::native_process::run(
            &image,
            &std::env::current_dir()?,
            &mut command,
            &mut environment,
        )?;
        assert_eq!(exit, 0);
        Ok(())
    }

    #[test]
    fn rejects_invalid_delimiters_and_nuls() {
        for (name, value) in [
            ("", "value"),
            ("=", "value"),
            ("A=B", "value"),
            ("=A=B", "value"),
            ("NAME\0SUFFIX", "value"),
            ("NAME", "value\0suffix"),
        ] {
            assert!(insert(&mut BTreeMap::new(), OsStr::new(name), OsStr::new(value)).is_err());
        }
    }

    #[test]
    fn empty_environment_is_double_nul_terminated() -> Result<(), Box<dyn Error>> {
        assert_eq!(encode(&BTreeMap::new())?, vec![0, 0]);
        Ok(())
    }
}
