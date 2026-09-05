//! PE and script dispatch over explicit resolver/manifest inputs; no persistent shim loader yet.
//! 使用显式解析器/清单输入进行 PE 与脚本分派；尚无持久化 shim 加载器。
//!
//! File-handle checks and a read lease do not replace handle-anchored ancestor protection or
//! a runtime-wide uninstall lease. Callers must trust registered source directories.
//! 文件句柄检查和读取租约不替代句柄锚定祖先防护或运行时级卸载租约。
//! 调用方必须信任已登记的来源目录。

mod environment;
mod scripts;

use crate::{
    commands::WindowsCommandCaseMapper,
    console::ConsoleWaitGuard,
    file_identity::{FileIdentity, file_identity},
    native_process, pe,
    state::{WindowsStateFileSystem, check_ancestors, check_type, io_error, normalize_path},
};
use pyrudder_core::{
    Error, ErrorKind, Result,
    commands::{
        CommandExtension, CommandKey, CommandKind, CommandOrigin, CommandRequest, CommandStatus,
        CommandTarget, RuntimeCommandManifest, lookup_command,
    },
    resolver::{ResolutionStep, ResolveRequest, VersionSource, resolve},
    runtime::{RuntimeId, RuntimeRecord},
    selector::RuntimeSelection,
    state::StateFileSystem,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

/// Inputs supplied by a future shim loader or an explicitly invoked development runner.
/// 由后续 shim 加载器或显式调用的开发运行入口提供的输入。
pub struct DispatchContext<'a> {
    /// Scope-resolution request; cwd also becomes the child's working directory.
    /// 作用域解析请求；cwd 同时作为子进程工作目录。
    pub scope: &'a ResolveRequest<'a>,
    /// Registered runtime metadata. / 已登记的运行时元数据。
    pub runtimes: &'a [RuntimeRecord],
    /// Per-runtime command inventories. / 每运行时命令清单。
    pub manifests: &'a [RuntimeCommandManifest],
    /// Explicit logical or filename identity, normally obtained from a shim index.
    /// 显式逻辑或文件名标识，通常从 shim 索引获取。
    pub command: &'a CommandRequest,
    /// Optional trusted shim directory, not inferred from an arbitrary executable's parent.
    /// 可选的可信 shim 目录，不从任意可执行文件的父目录推断。
    pub shims_dir: Option<&'a Path>,
    /// Captured environment; values remain in native encoding and the parent is never changed.
    /// 捕获的环境；值保留原生编码，绝不修改父进程。
    pub environment: &'a [(OsString, OsString)],
}

/// Full Windows exit code, distinct from a management error's small exit-code category.
/// 完整 Windows 退出码，与管理错误的小范围退出码类别区分。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeExitCode(u32);

impl NativeExitCode {
    /// Raw unsigned Windows exit code. / 原始无符号 Windows 退出码。
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Bit-preserving value for `std::process::exit` on Windows.
    /// Windows 上供 `std::process::exit` 使用、保留全部位的值。
    #[must_use]
    pub const fn for_process_exit(self) -> i32 {
        i32::from_ne_bytes(self.0.to_ne_bytes())
    }
}

/// Prepared target and optional system interpreter, held without write/delete sharing.
/// 已准备的目标和可选系统解释器，持有不共享写入和删除的句柄。
pub struct PreparedCommand {
    runtime_id: RuntimeId,
    source: VersionSource,
    trace: Vec<ResolutionStep>,
    target: CommandTarget,
    parent: PathBuf,
    cwd: PathBuf,
    file: File,
    identity: FileIdentity,
    environment: Vec<u16>,
    filtered_paths: Vec<PathBuf>,
    interpreter: Option<scripts::ScriptHost>,
}

/// Compatibility name; `prepare_native_dispatch` still accepts only PE targets.
/// 兼容类型名；`prepare_native_dispatch` 仍只接受 PE 目标。
pub type PreparedNativeCommand = PreparedCommand;

impl std::fmt::Debug for PreparedCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Environment values may contain secrets; never expose the encoded block through Debug.
        // 环境变量值可能包含秘密；绝不通过 Debug 暴露编码后的环境块。
        formatter
            .debug_struct("PreparedCommand")
            .field("runtime_id", &self.runtime_id)
            .field("source", &self.source)
            .field("target", &self.target.path)
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

impl PreparedCommand {
    /// Exact selected installation. / 精确选中的安装。
    #[must_use]
    pub const fn runtime_id(&self) -> &RuntimeId {
        &self.runtime_id
    }

    /// Selected target snapshot. / 选中的目标快照。
    #[must_use]
    pub const fn target(&self) -> &CommandTarget {
        &self.target
    }

    /// Scope which selected the runtime. / 选中运行时的作用域。
    #[must_use]
    pub const fn source(&self) -> &VersionSource {
        &self.source
    }

    /// Successful resolution trace. / 成功解析的来源诊断链。
    #[must_use]
    pub fn trace(&self) -> &[ResolutionStep] {
        &self.trace
    }

    /// PATH entries omitted from the child because they were managed, duplicate, or unsafe.
    /// 因属于受管目录、重复或不安全而从子进程 PATH 中移除的项。
    #[must_use]
    pub fn filtered_paths(&self) -> &[PathBuf] {
        &self.filtered_paths
    }

    /// Optional absolute system interpreter; native PE targets return `None`.
    /// 可选的系统解释器绝对路径；原生 PE 目标返回 `None`。
    #[must_use]
    pub fn interpreter(&self) -> Option<&Path> {
        self.interpreter.as_ref().map(scripts::ScriptHost::path)
    }

    /// Runs the selected target synchronously with inherited standard streams and console.
    /// 使用继承的标准流和控制台，同步执行选中的目标。
    ///
    /// PE arguments use CRT quoting. CMD/BAT use a restricted, separately quoted command;
    /// quotes, percent/exclamation signs, controls, and non-Unicode input are rejected.
    /// `PowerShell` uses `-NoLogo -NoProfile -File` without changing execution policy.
    /// Script hosts retain their own parameter, output, and exit semantics; no sandbox is provided.
    /// PE 参数使用 CRT 引用。CMD/BAT 使用受限的独立编码，拒绝引号、百分号/感叹号、
    /// 控制字符及非 Unicode 输入。`PowerShell` 使用 `-NoLogo -NoProfile -File`，
    /// 不改变执行策略。脚本宿主保留自身参数、输出和退出语义；不提供沙箱。
    ///
    /// # Errors
    /// Reports changed/replaced files, invalid arguments, startup/wait failures, or console
    /// handler contention. A nonzero child exit is returned unchanged, not treated as an error.
    /// 报告文件变化/替换、无效参数、启动/等待失败或控制台处理冲突。
    /// 子进程非零退出原样返回，不视为管理错误。
    pub fn run(mut self, arguments: &[OsString]) -> Result<NativeExitCode> {
        let mut command = if let Some(interpreter) = &self.interpreter {
            interpreter.command_line(&self.target.path, arguments)?
        } else {
            native_process::command_line(&self.target.path, arguments)?
        };
        recheck_file(&self.target.path, &self.parent, self.identity)?;
        validate_target(&mut self.file, &self.target)?;
        if let Some(interpreter) = &mut self.interpreter {
            interpreter.recheck()?;
        }
        let _console = if self.target.status == CommandStatus::Available(CommandKind::PeGui) {
            None
        } else {
            ConsoleWaitGuard::install()?
        };
        let image = self
            .interpreter
            .as_ref()
            .map_or(self.target.path.as_path(), scripts::ScriptHost::path);
        native_process::run(image, &self.cwd, &mut command, &mut self.environment)
            .map(NativeExitCode)
            .map_err(|error| process_error(image, &error))
    }
}

/// Resolves one active runtime, finds only its command, and verifies an existing native target.
/// 解析单个活动运行时，只查询其命令，并验证已有原生目标。
///
/// No interpreter or command is executed here. This compatibility entry rejects scripts;
/// use `prepare_dispatch` to opt into script adapters. System fallback is not implemented.
/// 此处不执行解释器或命令。此兼容入口拒绝脚本；使用 `prepare_dispatch` 启用脚本适配器。
/// 系统回退尚未实现。
///
/// # Errors
/// Reports resolution/manifest failures, invalid source boundaries, stale metadata,
/// reparse points, unsupported formats, self-recursion, or invalid child environment data.
/// 报告解析/清单失败、无效来源边界、过期元数据、重解析点、不支持格式、自递归或无效子环境数据。
pub fn prepare_native_dispatch(context: &DispatchContext<'_>) -> Result<PreparedNativeCommand> {
    prepare(context, true)
}

/// Resolves and leases a PE target or a script and its fixed system interpreter.
/// 解析并租用 PE 目标，或脚本及其固定系统解释器。
///
/// CMD/BAT use System32's `cmd.exe`; PS1 uses its Windows `PowerShell` 5.1 host.
/// Neither PATH, COMSPEC, nor `SystemRoot` selects the interpreter. Preparation is read-only;
/// argument checks finish in `PreparedCommand::run` before any process starts.
/// CMD/BAT 使用 System32 的 `cmd.exe`；PS1 使用其中的 Windows `PowerShell` 5.1 宿主。
/// 不从 PATH、COMSPEC 或 `SystemRoot` 选择解释器。准备阶段只读；参数检查在
/// `PreparedCommand::run` 中完成，且早于任何进程启动。
///
/// # Errors
/// Reports resolution, stale targets, unavailable interpreters, unsafe paths, or invalid
/// environment data. System fallback remains unsupported; no other runtime is searched.
/// 报告解析、过期目标、不可用解释器、不安全路径或无效环境数据错误。
/// 仍不支持系统回退；不会搜索其他运行时。
pub fn prepare_dispatch(context: &DispatchContext<'_>) -> Result<PreparedCommand> {
    prepare(context, false)
}

fn prepare(context: &DispatchContext<'_>, native_only: bool) -> Result<PreparedCommand> {
    if !crate::IS_SUPPORTED_HOST {
        return Err(Error::new(
            ErrorKind::Usage,
            "Process dispatch currently requires Windows x64",
        ));
    }
    let report = resolve(&WindowsStateFileSystem, context.scope, context.runtimes);
    let resolved = report.result?;
    let RuntimeSelection::Registered(runtime) = resolved.selection else {
        return Err(Error::new(
            ErrorKind::Usage,
            "System process dispatch is not implemented",
        ));
    };
    let target = lookup_command(context.manifests, runtime.id(), context.command)?.clone();
    let CommandStatus::Available(kind) = target.status else {
        return Err(invalid_target());
    };
    if native_only && !kind.is_pe() {
        return Err(Error::new(
            ErrorKind::Usage,
            "Native-only dispatch rejects scripts; use prepare_dispatch",
        ));
    }
    let allowed = match target.origin {
        CommandOrigin::Root => runtime.root(),
        CommandOrigin::Script(index) => runtime
            .command_dirs()
            .get(index)
            .ok_or_else(invalid_target)?
            .as_path(),
    };
    let parent = WindowsStateFileSystem.canonical_directory(allowed)?;
    let actual_parent = WindowsStateFileSystem
        .canonical_directory(target.path.parent().ok_or_else(invalid_target)?)?;
    if environment::path_key(&parent)? != environment::path_key(&actual_parent)? {
        return Err(invalid_target());
    }
    let mut file = open_target(&target.path)?;
    validate_target(&mut file, &target)?;
    let identity = file_identity(&file)
        .map_err(|error| io_error("identify command file", &target.path, &error))?;
    reject_router(identity)?;
    if let Some(shims) = context.shims_dir {
        let shims = WindowsStateFileSystem.normalize_directory(shims)?;
        if environment::path_key(&parent)?.starts_with(environment::path_key(&shims)?) {
            return Err(invalid_target());
        }
    }
    let cwd =
        environment::dos_path(&WindowsStateFileSystem.canonical_directory(context.scope.cwd)?);
    let (environment, filtered_paths) = environment::build(context, runtime)?;
    let interpreter = if kind.is_pe() {
        None
    } else {
        Some(scripts::ScriptHost::prepare(kind)?)
    };
    Ok(PreparedCommand {
        runtime_id: runtime.id().clone(),
        source: resolved.source,
        trace: report.steps,
        target,
        parent,
        cwd,
        file,
        identity,
        environment,
        filtered_paths,
        interpreter,
    })
}

/// Returns the current executable's actual filename key, not `argv[0]` or a guessed bare name.
/// 返回当前可执行文件实际文件名的键，不使用 `argv[0]` 或猜测的裸名。
///
/// A future shim index must translate this exact filename to its published request identity.
/// 后续 shim 索引必须将此精确文件名翻译为已发布的请求标识。
///
/// # Errors
/// Reports unavailable/invalid executable names or native casing failures.
/// 报告不可用/无效的可执行文件名或原生大小写映射失败。
pub fn current_image_key() -> Result<CommandKey> {
    let image = std::env::current_exe()
        .map_err(|_| Error::new(ErrorKind::Internal, "Cannot locate current executable"))?;
    let name = image
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(invalid_target)?;
    CommandKey::new(name, &WindowsCommandCaseMapper)
}

fn open_target(path: &Path) -> Result<File> {
    let path = normalize_path(path)?;
    check_ancestors(&path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::new(
                    ErrorKind::CommandMissing,
                    "Selected command no longer exists; refresh the manifest",
                )
            } else {
                io_error("lease command image", &path, &error)
            }
        })?;
    check_type(
        &file
            .metadata()
            .map_err(|error| io_error("inspect command handle", &path, &error))?,
        false,
    )?;
    Ok(file)
}

pub(crate) fn lease_target(path: &Path) -> Result<File> {
    open_target(path)
}

fn validate_target(file: &mut File, target: &CommandTarget) -> Result<()> {
    use std::{
        io::{Seek, SeekFrom},
        os::windows::fs::MetadataExt,
    };
    let metadata = file
        .metadata()
        .map_err(|error| io_error("inspect command metadata", &target.path, &error))?;
    check_type(&metadata, false)?;
    if metadata.len() != target.fingerprint.size
        || metadata.file_attributes() != target.fingerprint.attributes
        || metadata
            .modified()
            .map_err(|error| io_error("read command timestamp", &target.path, &error))?
            != target.fingerprint.modified
    {
        return Err(Error::new(
            ErrorKind::BrokenRuntime,
            "Command metadata changed; refresh the manifest",
        ));
    }
    let CommandStatus::Available(expected) = target.status else {
        return Err(invalid_target());
    };
    if !expected.is_pe() {
        // Script classification is extension-only; do not read, rewrite, or evaluate its contents.
        // 脚本只按扩展名分类；不读取、重写或求值其内容。
        return match (target.extension, expected) {
            (CommandExtension::Cmd, CommandKind::Cmd)
            | (CommandExtension::Bat, CommandKind::Bat)
            | (CommandExtension::PowerShell, CommandKind::PowerShell) => Ok(()),
            _ => Err(invalid_target()),
        };
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek command header", &target.path, &error))?;
    let kind = pe::classify(file, metadata.len())
        .map_err(|error| io_error("recheck command header", &target.path, &error))?
        .map_err(|_| {
            Error::new(
                ErrorKind::BrokenRuntime,
                "Selected image has an unsupported or damaged PE header",
            )
        })?;
    if target.status != CommandStatus::Available(kind) {
        return Err(invalid_target());
    }
    Ok(())
}

fn recheck_file(path: &Path, parent: &Path, identity: FileIdentity) -> Result<()> {
    let current_parent =
        WindowsStateFileSystem.canonical_directory(path.parent().ok_or_else(invalid_target)?)?;
    if environment::path_key(&current_parent)? != environment::path_key(parent)? {
        return Err(invalid_target());
    }
    let current = open_target(path)?;
    if file_identity(&current)
        .map_err(|error| io_error("recheck command identity", path, &error))?
        != identity
    {
        return Err(Error::new(
            ErrorKind::BrokenRuntime,
            "Command file was replaced after preparation",
        ));
    }
    Ok(())
}

fn reject_router(identity: FileIdentity) -> Result<()> {
    let own_path = std::env::current_exe()
        .map_err(|_| Error::new(ErrorKind::Internal, "Cannot locate current executable"))?;
    let own = File::open(&own_path)
        .map_err(|error| io_error("open current executable", &own_path, &error))?;
    if identity
        == file_identity(&own)
            .map_err(|error| io_error("identify current executable", &own_path, &error))?
    {
        return Err(Error::new(
            ErrorKind::Conflict,
            "Command points to the current router executable",
        ));
    }
    Ok(())
}

fn invalid_target() -> Error {
    Error::new(
        ErrorKind::BrokenRuntime,
        "Command target does not match its registered source or manifest",
    )
}

fn process_error(path: &Path, error: &std::io::Error) -> Error {
    if matches!(error.raw_os_error(), Some(193 | 216)) {
        Error::new(
            ErrorKind::BrokenRuntime,
            "Windows could not load the selected native image",
        )
    } else if error.kind() == std::io::ErrorKind::InvalidInput || error.raw_os_error() == Some(206)
    {
        Error::new(
            ErrorKind::Usage,
            "Invalid or oversized native process input",
        )
    } else {
        io_error("start or wait for command", path, error)
    }
}
