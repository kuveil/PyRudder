//! Installed shim routing and explicit execution over coherent persisted state.
//! 基于一致持久状态的已安装 shim 路由与显式执行。

use crate::{
    commands::{WindowsCommandCaseMapper, discover_commands},
    process::{DispatchContext, NativeExitCode, current_image_key, prepare_dispatch},
    publication,
    registry::{Location, Registry, Snapshot},
    session,
    state::WindowsStateFileSystem,
    storage::{DirectoryLease, FileLease, invalid, path_key, sha256_file},
};
use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{CommandKey, CommandRequest},
    resolver::{ResolveRequest, resolve},
    selector::RuntimeSelection,
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Executes the actual shim filename from its colocated location/index contract.
/// 根据同目录位置/索引契约执行实际 shim 文件名。
///
/// # Errors
/// Rejects unpublished, relocated, changed, or subsystem-mismatched shims and routing failures.
/// 拒绝未发布、被移动、被修改或子系统不匹配的 shim 及路由错误。
pub fn run_shim(gui: bool) -> Result<NativeExitCode> {
    let image = std::env::current_exe().map_err(|_| invalid("Cannot locate shim image"))?;
    let directory = image
        .parent()
        .ok_or_else(|| invalid("Shim has no parent"))?;
    let _anchor = DirectoryLease::acquire(directory, false)?;
    let location = Location::read(directory)?;
    if path_key(directory)? != path_key(&location.shims_dir)? {
        return Err(invalid("Shim was moved outside its published directory"));
    }
    let registry = Registry::new(&location.config_dir)?;
    let snapshot = registry.load()?;
    let key = current_image_key()?;
    let entry = snapshot
        .shims
        .iter()
        .find(|entry| {
            CommandKey::new(&entry.filename, &WindowsCommandCaseMapper)
                .is_ok_and(|candidate| candidate == key)
        })
        .ok_or_else(|| {
            Error::new(
                ErrorKind::CommandMissing,
                "This shim is no longer published; run pyrudder rehash",
            )
        })?;
    if entry.gui != gui || sha256_file(&image, 128 * 1024 * 1024)? != entry.sha256 {
        return Err(Error::new(
            ErrorKind::Integrity,
            "Shim bytes or subsystem differ from the published index",
        ));
    }
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    execute(&location, &entry.request()?, None, &arguments)
}

/// Executes an indexed explicit wrapper name; no target path comes from wrapper arguments.
/// 执行已索引的显式包装器名称；包装器参数不能提供目标路径。
///
/// # Errors
/// Rejects missing wrapper entries or dispatcher ownership mismatches.
/// 拒绝缺失包装器入口或分派器所有权不一致。
pub fn run_wrapper(filename: &str, arguments: &[OsString]) -> Result<NativeExitCode> {
    let image = std::env::current_exe().map_err(|_| invalid("Cannot locate dispatcher"))?;
    let directory = image
        .parent()
        .ok_or_else(|| invalid("Dispatcher has no parent"))?;
    let location = Location::read(directory)?;
    if path_key(directory)? != path_key(&location.shims_dir)? {
        return Err(invalid(
            "Internal wrapper dispatcher must run from the published shim directory",
        ));
    }
    let snapshot = Registry::new(&location.config_dir)?.load()?;
    let manager = snapshot
        .shims
        .iter()
        .find(|entry| entry.filename.eq_ignore_ascii_case("pyrudder-dispatch.exe"))
        .ok_or_else(|| invalid("Wrapper dispatcher is not published"))?;
    if sha256_file(&image, 128 * 1024 * 1024)? != manager.sha256 {
        return Err(Error::new(
            ErrorKind::Integrity,
            "Wrapper dispatcher was modified",
        ));
    }
    let key = CommandKey::new(filename, &WindowsCommandCaseMapper)?;
    let entry = snapshot
        .shims
        .iter()
        .find(|entry| {
            CommandKey::new(&entry.filename, &WindowsCommandCaseMapper)
                .is_ok_and(|candidate| candidate == key)
        })
        .ok_or_else(|| {
            Error::new(
                ErrorKind::CommandMissing,
                "Wrapper is no longer in the published index",
            )
        })?;
    execute(&location, &entry.request()?, None, arguments)
}

/// Executes one registered command with runtime/ancestor leases and post-command rehash.
/// 使用运行时/祖先租约执行一个已登记命令，并在命令结束后刷新索引。
///
/// # Errors
/// Propagates selection and startup failures. A refresh failure never replaces a child's exit code.
/// 传递选择和启动失败；刷新失败绝不覆盖子进程退出码。
pub fn execute(
    location: &Location,
    command: &CommandRequest,
    explicit: Option<&str>,
    arguments: &[OsString],
) -> Result<NativeExitCode> {
    location.validate()?;
    let _installation_access = crate::installation::shared_access(location)?;
    let registry = Registry::new(&location.config_dir)?;
    let cwd = std::env::current_dir().map_err(|_| invalid("Cannot read working directory"))?;
    let shell = if explicit.is_some() {
        None
    } else {
        session::selection_value(&location.config_dir)?
    };
    let global = location.config_dir.join("global-version");
    let scope = ResolveRequest {
        explicit,
        shell_version: shell.as_deref(),
        cwd: &cwd,
        workspace_boundary: None,
        global_file: &global,
        system_fallback: false,
    };
    let initial = registry.load()?;
    let resolved = resolve(&WindowsStateFileSystem, &scope, &initial.runtimes).result?;
    let RuntimeSelection::Registered(selected) = resolved.selection else {
        return Err(invalid(
            "System forwarding is not enabled for this installation",
        ));
    };
    let id = selected.id().clone();
    let _lease = FileLease::acquire(&registry.lease_path(&id), false, false)?;
    // Re-read after acquiring the lease so removal cannot race a stale registry read.
    // 获取租约后重新读取，避免删除操作与过期登记读取竞争。
    let snapshot = registry.load()?;
    let record = snapshot
        .runtimes
        .iter()
        .find(|record| record.id() == &id)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::NotInstalled,
                "Runtime was unregistered before dispatch",
            )
        })?;
    let _root = DirectoryLease::acquire(record.root(), false)?;
    let environment = std::env::vars_os().collect::<Vec<_>>();
    let fixed_id = id.to_string();
    let fixed = ResolveRequest {
        explicit: Some(&fixed_id),
        ..scope
    };
    let prepared = prepare_dispatch(&DispatchContext {
        scope: &fixed,
        runtimes: &snapshot.runtimes,
        manifests: &snapshot.manifests,
        command,
        shims_dir: Some(&location.shims_dir),
        environment: &environment,
    })?;
    let _source = DirectoryLease::acquire(
        prepared
            .target()
            .path
            .parent()
            .ok_or_else(|| invalid("Command lacks a source directory"))?,
        false,
    )?;
    let _interpreter = prepared
        .interpreter()
        .map(|path| {
            DirectoryLease::acquire(
                path.parent()
                    .ok_or_else(|| invalid("Interpreter lacks a directory"))?,
                false,
            )
        })
        .transpose()?;
    let exit = prepared.run(arguments)?;
    if let Err(error) = refresh_after(location, &registry, &snapshot, &id) {
        eprintln!(
            "PyRudder: command finished, but command-index refresh failed: {error}\nRun pyrudder rehash to retry."
        );
    }
    Ok(exit)
}

fn refresh_after(
    location: &Location,
    registry: &Registry,
    snapshot: &Snapshot,
    id: &pyrudder_core::runtime::RuntimeId,
) -> Result<()> {
    let record = snapshot
        .runtimes
        .iter()
        .find(|record| record.id() == id)
        .ok_or_else(|| invalid("Missing runtime after dispatch"))?;
    let refreshed = discover_commands(record)?;
    if snapshot
        .manifests
        .iter()
        .find(|manifest| manifest.runtime_id() == id)
        .is_some_and(|previous| previous.directories() == refreshed.directories())
    {
        return Ok(());
    }
    let mut transaction = registry.transaction()?;
    let current = transaction
        .snapshot
        .runtimes
        .iter()
        .find(|record| record.id() == id)
        .ok_or_else(|| invalid("Runtime disappeared during refresh"))?;
    let refreshed = discover_commands(current)?;
    transaction
        .snapshot
        .manifests
        .retain(|manifest| manifest.runtime_id() != id);
    transaction.snapshot.manifests.push(refreshed);
    publication::publish(location, transaction)?;
    Ok(())
}

/// Builds a logical/exact request from a CLI command token.
/// 根据 CLI 命令项构建逻辑或精确请求。
///
/// # Errors
/// Rejects paths and invalid filenames. / 拒绝路径和无效文件名。
pub fn command_request(name: &str) -> Result<CommandRequest> {
    let key = CommandKey::new(name, &WindowsCommandCaseMapper)?;
    Ok(
        if pyrudder_core::commands::CommandExtension::split(name).is_some() {
            CommandRequest::Exact(key)
        } else {
            CommandRequest::Bare(key)
        },
    )
}

/// Finds the closest location file beside the currently running manager.
/// 查找当前管理程序同目录的位置文件。
///
/// # Errors
/// Reports unavailable executable paths. / 报告不可用的可执行文件路径。
pub fn current_program_directory() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|_| invalid("Cannot locate current executable"))?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| invalid("Executable has no parent"))
}
