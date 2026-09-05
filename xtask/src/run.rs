//! Explicit manual PE/script runner; no persisted records or Python health probing.
//! 显式手动 PE/脚本运行入口；不持久化记录或探测 Python 健康状态。

use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{CommandKey, CommandRequest},
    resolver::ResolveRequest,
    runtime::{InstallationId, RuntimeHealth, RuntimeId, RuntimeOrigin, RuntimeRecord},
    state::StateFileSystem,
};
use pyrudder_platform_windows::{
    commands::{WindowsCommandCaseMapper, discover_commands},
    process::{DispatchContext, NativeExitCode, prepare_dispatch},
    state::WindowsStateFileSystem,
};
use std::{ffi::OsString, path::PathBuf};

struct RunArguments {
    version: String,
    root: PathBuf,
    command: String,
    exact: bool,
    scripts: Vec<PathBuf>,
    forwarded: Vec<OsString>,
}

impl RunArguments {
    fn parse(arguments: Vec<OsString>) -> Result<Self> {
        let mut arguments = arguments.into_iter();
        let version = text(arguments.next())?;
        let root = PathBuf::from(arguments.next().ok_or_else(usage)?);
        let command = text(arguments.next())?;
        let mut exact = false;
        let mut scripts = Vec::new();
        while let Some(argument) = arguments.next() {
            if argument == "--" {
                return Ok(Self {
                    version,
                    root,
                    command,
                    exact,
                    scripts,
                    forwarded: arguments.collect(),
                });
            }
            if argument == "--exact" && !exact {
                exact = true;
            } else if argument == "--scripts" {
                scripts.push(PathBuf::from(arguments.next().ok_or_else(usage)?));
            } else {
                return Err(usage());
            }
        }
        Err(usage())
    }
}

pub(super) fn execute(arguments: Vec<OsString>) -> Result<NativeExitCode> {
    let arguments = RunArguments::parse(arguments)?;
    let root = WindowsStateFileSystem.canonical_directory(&arguments.root)?;
    let scripts = if arguments.scripts.is_empty() {
        vec![root.join("Scripts")]
    } else {
        arguments.scripts
    };
    // This fixed identity exists only in one invocation's in-memory development fixture.
    // The user explicitly supplies and authorizes this root; Ready is not a completed health probe.
    // 固定标识只存在于单次调用的内存开发夹具中。
    // 用户显式提供并授权此根目录；Ready 不代表完成了真实健康探测。
    let id = RuntimeId::new(arguments.version.parse()?)?.with_installation(InstallationId::new(1)?);
    let mut record = RuntimeRecord::new(
        id,
        root.clone(),
        scripts,
        RuntimeOrigin::External {
            registered_executable: root.join("python.exe"),
        },
    )?;
    record.set_health(RuntimeHealth::Ready);
    let manifest = discover_commands(&record)?;
    let selected = record.id().to_string();
    let cwd = std::env::current_dir()
        .map_err(|_| Error::new(ErrorKind::Internal, "Cannot read current working directory"))?;
    let global = root.join("unused-development-selection");
    let scope = ResolveRequest {
        explicit: Some(&selected),
        shell_version: None,
        cwd: &cwd,
        workspace_boundary: None,
        global_file: &global,
        system_fallback: false,
    };
    let key = CommandKey::new(&arguments.command, &WindowsCommandCaseMapper)?;
    let command = if arguments.exact {
        CommandRequest::Exact(key)
    } else {
        CommandRequest::Bare(key)
    };
    let runtimes = [record];
    let manifests = [manifest];
    let environment = std::env::vars_os().collect::<Vec<_>>();
    let prepared = prepare_dispatch(&DispatchContext {
        scope: &scope,
        runtimes: &runtimes,
        manifests: &manifests,
        command: &command,
        shims_dir: None,
        environment: &environment,
    })?;
    prepared.run(&arguments.forwarded)
}

fn text(value: Option<OsString>) -> Result<String> {
    value.ok_or_else(usage)?.into_string().map_err(|_| usage())
}
fn usage() -> Error {
    Error::new(
        ErrorKind::Usage,
        "Expected: xtask run <version> <absolute-root> <command> [--exact] [--scripts <absolute-dir>]... -- [args...]",
    )
}
