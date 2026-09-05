//! Active-runtime resolution types.
//! 活动运行时解析类型。

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use crate::{
    Error, ErrorKind, Result,
    runtime::RuntimeRecord,
    selector::{RuntimeSelection, VersionSelector},
    state::{StateFileSystem, check_system_policy, read_selection_file},
};

/// The source that selected an active runtime.
/// 选中活动运行时的来源。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionSource {
    /// An explicit command-line selection.
    /// 命令行中的显式选择。
    Explicit,
    /// The `PYRUDDER_VERSION` environment variable.
    /// `PYRUDDER_VERSION` 环境变量。
    ShellEnvironment,
    /// The closest valid project `.python-version` file.
    /// 距离当前目录最近的合法项目 `.python-version` 文件。
    LocalFile(PathBuf),
    /// The user's global selection file.
    /// 用户的全局选择文件。
    GlobalFile(PathBuf),
    /// An explicitly enabled system fallback.
    /// 用户显式启用的系统回退。
    SystemFallback,
}

/// Maximum directories visited by one project-file search.
/// 单次项目文件查找最多访问的目录数。
pub const MAX_LOCAL_DEPTH: usize = 256;

/// Inputs captured once for one resolution; no ambient environment is read here.
/// 单次解析捕获的输入；此处不读取外部进程环境。
#[derive(Clone, Copy, Debug)]
pub struct ResolveRequest<'a> {
    /// Highest-priority explicit selector. / 最高优先级的显式选择器。
    pub explicit: Option<&'a str>,
    /// Current-shell selector, kept in native encoding until reached.
    /// 当前终端选择器，在解析到此来源前保留原生编码。
    pub shell_version: Option<&'a OsStr>,
    /// Existing starting directory. / 已存在的查找起始目录。
    pub cwd: &'a Path,
    /// Optional inclusive ancestor boundary; absence means filesystem root.
    /// 可选且包含自身的祖先边界；缺省表示文件系统根目录。
    pub workspace_boundary: Option<&'a Path>,
    /// Absolute user global-selection file. / 用户全局选择文件的绝对路径。
    pub global_file: &'a Path,
    /// Trusted opt-in policy; projects cannot enable system fallback.
    /// 可信的显式启用策略；项目不能自行启用系统回退。
    pub system_fallback: bool,
}

/// Outcome of a source that was actually consulted; lower sources are not eagerly read.
/// 实际检查过的来源结果；不会提前读取低优先级来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceOutcome {
    /// No value/file was present. / 没有值或文件。
    Absent,
    /// This source selected the result. / 此来源选中了结果。
    Selected,
    /// This source failed and stopped resolution. / 此来源失败并终止解析。
    Rejected,
}

/// One ordered explain entry. / 一条有序来源诊断记录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionStep {
    /// Source checked, including the exact file path. / 检查的来源，包含精确文件路径。
    pub source: VersionSource,
    /// Presence or selection outcome. / 存在性或选择结果。
    pub outcome: TraceOutcome,
}

/// Selected metadata and provenance; not an executable dispatch authorization.
/// 选中的元数据及来源；不构成可执行文件分派授权。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRuntime<'a> {
    /// Registered metadata or policy-allowed system intent. / 登记元数据或策略允许的系统意图。
    pub selection: RuntimeSelection<'a>,
    /// Winning source. / 命中来源。
    pub source: VersionSource,
    /// Validated selector supplied by the winning source. / 命中来源提供的合法选择器。
    pub selector: VersionSelector,
}

/// A result with its explain trace, retained even when resolution fails.
/// 结果与来源诊断链；解析失败时也保留诊断链。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionReport<'a> {
    /// Checked sources in descending priority and project proximity.
    /// 按优先级和项目距离排列的已检查来源。
    pub steps: Vec<ResolutionStep>,
    /// Resolution success or the first blocking error. / 解析成功或首个阻断错误。
    pub result: Result<ResolvedRuntime<'a>>,
}

/// Resolves explicit > shell > nearest local > global > opted-in system.
/// 按显式参数 > shell > 最近 local > global > 显式启用的 system 解析。
///
/// Invalid or unhealthy higher-priority choices stop resolution. Nothing is downloaded,
/// executed, written, or changed in the parent shell. Dispatch-time probes remain mandatory.
/// 高优先级选择无效或不健康时终止解析。不下载、不执行、不写入，也不修改父 shell。
/// 分派时仍须进行文件与目标探测。
#[must_use]
pub fn resolve<'a>(
    fs: &impl StateFileSystem,
    request: &ResolveRequest<'_>,
    runtimes: &'a [RuntimeRecord],
) -> ResolutionReport<'a> {
    let mut steps = Vec::new();
    let result = resolve_inner(fs, request, runtimes, &mut steps);
    ResolutionReport { steps, result }
}

fn resolve_inner<'a>(
    fs: &impl StateFileSystem,
    request: &ResolveRequest<'_>,
    runtimes: &'a [RuntimeRecord],
    steps: &mut Vec<ResolutionStep>,
) -> Result<ResolvedRuntime<'a>> {
    if let Some(value) = request.explicit {
        return select_source(
            VersionSource::Explicit,
            value.parse(),
            request.system_fallback,
            runtimes,
            steps,
        );
    }
    absent(steps, VersionSource::Explicit);
    if let Some(value) = request.shell_version {
        let selector = value
            .to_str()
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Usage,
                    "PYRUDDER_VERSION must contain a valid ASCII selector",
                )
            })
            .and_then(str::parse);
        return select_source(
            VersionSource::ShellEnvironment,
            selector,
            request.system_fallback,
            runtimes,
            steps,
        );
    }
    absent(steps, VersionSource::ShellEnvironment);

    let directories = match search_directories(fs, request.cwd, request.workspace_boundary) {
        Ok(directories) => directories,
        Err(error) => {
            return select_source(
                VersionSource::LocalFile(request.cwd.join(".python-version")),
                Err(error),
                request.system_fallback,
                runtimes,
                steps,
            );
        }
    };
    for (depth, directory) in directories.into_iter().enumerate() {
        let path = directory.join(".python-version");
        let source = VersionSource::LocalFile(path.clone());
        if depth == MAX_LOCAL_DEPTH {
            let error = Error::new(ErrorKind::Usage, "Project search exceeds 256 directories")
                .with_hint("Set a closer explicit workspace boundary");
            return select_source(source, Err(error), request.system_fallback, runtimes, steps);
        }
        match read_selection_file(fs, &path).transpose() {
            None => absent(steps, source),
            Some(candidate) => {
                return select_source(source, candidate, request.system_fallback, runtimes, steps);
            }
        }
    }
    let source = VersionSource::GlobalFile(request.global_file.to_path_buf());
    match read_selection_file(fs, request.global_file).transpose() {
        None => absent(steps, source),
        Some(candidate) => {
            return select_source(source, candidate, request.system_fallback, runtimes, steps);
        }
    }
    if request.system_fallback {
        return select_source(
            VersionSource::SystemFallback,
            Ok(VersionSelector::System),
            true,
            runtimes,
            steps,
        );
    }
    absent(steps, VersionSource::SystemFallback);
    Err(
        Error::new(ErrorKind::NotInstalled, "No active runtime is configured").with_hint(
            "Select a registered runtime using an explicit, shell, local, or global choice",
        ),
    )
}

fn search_directories(
    fs: &impl StateFileSystem,
    cwd: &Path,
    boundary: Option<&Path>,
) -> Result<Vec<PathBuf>> {
    let cwd = fs.canonical_directory(cwd)?;
    let boundary = boundary
        .map(|path| fs.canonical_directory(path))
        .transpose()?;
    if boundary
        .as_ref()
        .is_some_and(|boundary| !cwd.ancestors().any(|ancestor| ancestor == boundary))
    {
        return Err(Error::new(
            ErrorKind::Usage,
            "Workspace boundary must contain the current directory",
        ));
    }
    let mut directories = Vec::new();
    // Include one sentinel entry so the caller fails only if it actually exhausts the search.
    // 包含一个哨兵条目，确保只有调用方实际耗尽查找上限时才报错。
    for directory in cwd.ancestors().take(MAX_LOCAL_DEPTH + 1) {
        directories.push(directory.to_path_buf());
        if boundary.as_deref() == Some(directory) {
            break;
        }
    }
    Ok(directories)
}

fn absent(steps: &mut Vec<ResolutionStep>, source: VersionSource) {
    steps.push(ResolutionStep {
        source,
        outcome: TraceOutcome::Absent,
    });
}

fn select_source<'a>(
    source: VersionSource,
    selector: Result<VersionSelector>,
    allow_system: bool,
    runtimes: &'a [RuntimeRecord],
    steps: &mut Vec<ResolutionStep>,
) -> Result<ResolvedRuntime<'a>> {
    let result = selector.and_then(|selector| {
        if matches!(selector, VersionSelector::System) {
            check_system_policy(allow_system)?;
        }
        let selection = selector.select(runtimes)?;
        Ok(ResolvedRuntime {
            selection,
            source: source.clone(),
            selector,
        })
    });
    steps.push(ResolutionStep {
        source,
        outcome: if result.is_ok() {
            TraceOutcome::Selected
        } else {
            TraceOutcome::Rejected
        },
    });
    result
}
