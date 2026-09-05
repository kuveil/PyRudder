//! Explicit isolated Python identity probing with bounded output and a deadline.
//! 显式隔离 Python 身份探测，限制输出量和完成期限。

use crate::{
    pe,
    state::{WindowsStateFileSystem, io_error},
    storage::{DirectoryLease, invalid, path_key},
};
use pyrudder_core::{Error, ErrorKind, Result, commands::CommandKind, state::StateFileSystem};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const SCRIPT: &str = "import json,sys,sysconfig,struct;print(json.dumps(dict(version='.'.join(map(str,sys.version_info[:3])),releaselevel=sys.version_info.releaselevel,implementation=sys.implementation.name,bits=struct.calcsize('P')*8,executable=sys.executable,prefix=sys.prefix,base_prefix=sys.base_prefix,scripts=[sysconfig.get_path('scripts')],debug=bool(sysconfig.get_config_var('Py_DEBUG')),free_threaded=bool(sysconfig.get_config_var('Py_GIL_DISABLED')))))";

/// Identity returned by the explicitly supplied interpreter, not a PATH search.
/// 显式指定解释器返回的身份，不来自 PATH 搜索。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PythonIdentity {
    /// Numeric exact version. / 数字精确版本。
    pub version: String,
    /// `CPython` release level. / `CPython` 发布阶段。
    pub releaselevel: String,
    /// Implementation name. / 实现名称。
    pub implementation: String,
    /// Pointer width. / 指针宽度。
    pub bits: u32,
    /// Reported interpreter path. / 报告的解释器路径。
    pub executable: PathBuf,
    /// Active Python prefix. / 活动 Python 前缀。
    pub prefix: PathBuf,
    /// Base interpreter prefix. / 基础解释器前缀。
    pub base_prefix: PathBuf,
    /// sysconfig script directories. / sysconfig 脚本目录。
    pub scripts: Vec<PathBuf>,
    /// Debug build marker. / 调试构建标记。
    pub debug: bool,
    /// Free-threaded build marker. / 自由线程构建标记。
    pub free_threaded: bool,
}

/// Probes only an explicitly trusted absolute Python EXE; never installs anything.
/// 只探测显式可信的绝对 Python EXE，不安装任何内容。
///
/// # Errors
/// Rejects non-x64/nonstandard Python, venvs unless allowed, changed paths, timeout, or malformed output.
/// 拒绝非 x64/非标准 Python、未允许的 venv、变化路径、超时或无效输出。
pub fn inspect(executable: &Path, allow_venv: bool) -> Result<PythonIdentity> {
    let parent = executable
        .parent()
        .ok_or_else(|| invalid("Interpreter requires an absolute parent"))?;
    let _anchor = DirectoryLease::acquire(parent, false)?;
    let executable = crate::state::normalize_path(executable)?;
    if executable
        .extension()
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
    {
        return Err(invalid("Registration requires a native .exe interpreter"));
    }
    let mut file = crate::process::lease_target(&executable)?;
    let size = file
        .metadata()
        .map_err(|error| io_error("inspect Python", &executable, &error))?
        .len();
    if pe::classify(&mut file, size)
        .map_err(|error| io_error("classify Python", &executable, &error))?
        != Ok(CommandKind::PeConsole)
    {
        return Err(Error::new(
            ErrorKind::BrokenRuntime,
            "Python must be a native x64 console PE, not a Store alias",
        ));
    }
    let mut command = Command::new(&executable);
    command
        .args(["-I", "-c", SCRIPT])
        .current_dir(parent)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .env_remove("PYTHONSTARTUP")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|error| io_error("start Python probe", &executable, &error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("Missing probe stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("Missing probe stderr"))?;
    let output = bounded_reader(stdout);
    let errors = bounded_reader(stderr);
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _killed = child.kill();
                let _waited = child.wait();
                return Err(Error::new(
                    ErrorKind::BrokenRuntime,
                    "Python probe failed or exceeded 15 seconds",
                ));
            }
        }
    };
    let bytes = output
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| invalid("Python probe output remained open"))?
        .map_err(|_| invalid("Cannot read Python probe output"))?;
    let error_bytes = errors
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| invalid("Python probe stderr remained open"))?
        .map_err(|_| invalid("Cannot read Python probe stderr"))?;
    if !status.success() || bytes.len() > 65_536 || error_bytes.len() > 65_536 {
        return Err(Error::new(
            ErrorKind::BrokenRuntime,
            "Python probe failed or produced excessive output",
        ));
    }
    let mut identity: PythonIdentity = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("Python probe did not return the expected identity JSON"))?;
    validate_identity(&mut identity, &executable, allow_venv)?;
    Ok(identity)
}

fn validate_identity(
    identity: &mut PythonIdentity,
    executable: &Path,
    allow_venv: bool,
) -> Result<()> {
    if identity.bits != 64
        || identity.implementation != "cpython"
        || identity.releaselevel != "final"
        || identity.debug
        || identity.free_threaded
    {
        return Err(invalid(
            "Only stable standard CPython x64 is currently supported",
        ));
    }
    let _version: pyrudder_core::version::PythonVersion = identity.version.parse()?;
    if path_key(&identity.executable)? != path_key(executable)? {
        return Err(invalid("Python reports a different executable path"));
    }
    if !allow_venv && path_key(&identity.prefix)? != path_key(&identity.base_prefix)? {
        return Err(invalid(
            "Virtual environment registration requires --allow-venv",
        ));
    }
    identity.prefix = WindowsStateFileSystem.canonical_directory(&identity.prefix)?;
    identity.base_prefix = WindowsStateFileSystem.canonical_directory(&identity.base_prefix)?;
    identity.executable = executable.to_path_buf();
    if identity.scripts.is_empty() || identity.scripts.len() > 63 {
        return Err(invalid("Python returned invalid script directories"));
    }
    for directory in &mut identity.scripts {
        *directory = WindowsStateFileSystem.normalize_directory(directory)?;
    }
    Ok(())
}

fn bounded_reader(reader: impl Read + Send + 'static) -> mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.take(65_537).read_to_end(&mut bytes).map(|_| bytes);
        let _sent = sender.send(result);
    });
    receiver
}

/// Resolves a supplied directory or file to its normalized Python executable.
/// 将指定目录或文件解析为规范 Python 可执行文件。
///
/// # Errors
/// Rejects relative paths, reparse points, and missing directories.
/// 拒绝相对路径、重解析点和缺失目录。
pub fn executable_path(path: &Path) -> Result<PathBuf> {
    let path = crate::state::normalize_path(path)?;
    let path = if path.is_dir() {
        WindowsStateFileSystem
            .canonical_directory(&path)?
            .join("python.exe")
    } else {
        path
    };
    let parent = WindowsStateFileSystem.canonical_directory(
        path.parent()
            .ok_or_else(|| invalid("Missing interpreter parent"))?,
    )?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| invalid("Missing interpreter filename"))?,
    ))
}
