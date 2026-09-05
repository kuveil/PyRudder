//! Typed CLI grammar; forwarded program arguments stay in native encoding.
//! 类型化 CLI 语法；转发给程序的参数保留原生编码。

use clap::{Parser, Subcommand, ValueEnum};
use std::{ffi::OsString, path::PathBuf};

#[derive(Parser)]
// Independent CLI switches are intentionally flags, not a domain state machine.
// 独立 CLI 开关有意使用标志，而不是领域状态机。
#[allow(clippy::struct_excessive_bools)]
#[command(
    name = "pyrudder",
    bin_name = "pyrudder",
    version,
    about = "Python runtimes, ordinary commands. / 切换 Python，照常输入命令。",
    disable_help_subcommand = true
)]
pub(crate) struct Arguments {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[arg(long, global = true)]
    pub verbose: bool,
    #[arg(long, global = true)]
    pub no_color: bool,
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub install_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub shims_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub runtimes_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub downloads_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub temp_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Action>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Shell {
    Powershell,
    Cmd,
}

#[derive(Subcommand)]
pub(crate) enum PathAction {
    /// Add bin and shims to system PATH. / 将 bin 和 shims 加入系统 PATH。
    Add,
    /// Remove only entries previously added here. / 仅移除本安装曾添加的项。
    Remove,
}

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Install the CLI and initialize local state. / 安装 CLI 并初始化本地状态。
    Setup {
        #[arg(long)]
        add_to_path: bool,
    },
    /// Manage this installation's system PATH. / 管理本安装的系统 PATH。
    Path {
        #[command(subcommand)]
        action: PathAction,
    },
    /// Register an explicitly trusted existing Python. / 登记显式可信的已有 Python。
    Register {
        path: PathBuf,
        #[arg(long)]
        alias: Option<String>,
        #[arg(long)]
        scan: bool,
        #[arg(long)]
        allow_venv: bool,
    },
    /// Remove a registration, never external Python files. / 移除登记，绝不删除外部 Python 文件。
    Unregister {
        version: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        yes: bool,
    },
    /// List installed runtimes. / 列出已登记运行时。
    List,
    /// Choose an official Python version to install. / 选择并安装官方 Python 版本。
    Available {
        #[arg(long)]
        offline: bool,
        /// Print the list without prompting or installing. / 仅打印列表，不交互或安装。
        #[arg(long)]
        list: bool,
    },
    /// Download and verify a runtime without installing it. / 只下载并验证运行时，不安装。
    Download {
        #[arg(default_value = "latest")]
        version: String,
        #[arg(long)]
        offline: bool,
    },
    /// Install an official runtime into the managed directory. / 安装官方运行时到托管目录。
    Install {
        #[arg(default_value = "latest")]
        version: String,
        #[arg(long)]
        alias: Option<String>,
        #[arg(long)]
        offline: bool,
    },
    /// Delete only an owned managed runtime. / 仅删除拥有所有权的托管运行时。
    Uninstall {
        version: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Inspect cache or delete one explicitly named cache file. / 检查缓存或删除一个显式指定的缓存文件。
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Set or show the global selection. / 设置或显示全局选择。
    Global {
        version: Option<String>,
        #[arg(long, conflicts_with = "version")]
        unset: bool,
    },
    /// Pin or show the current project's selection. / 固定或显示当前项目选择。
    Local {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// Select Python for this terminal without shell scripts. / 无需脚本即可选择当前终端 Python。
    #[command(alias = "use")]
    Shell {
        version: Option<String>,
        #[arg(long)]
        unset: bool,
    },
    /// Explain active runtime selection. / 解释活动运行时选择。
    Current {
        #[arg(long)]
        explain: bool,
    },
    /// Show the selected command target. / 显示所选命令目标。
    Which {
        command: String,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        explain: bool,
    },
    /// Show runtime metadata. / 显示运行时元数据。
    Info { version: String },
    /// List all indexed runtime commands. / 列出运行时的所有索引命令。
    Commands { version: Option<String> },
    /// Execute a command with an optional explicit selection. / 使用可选显式版本执行命令。
    Exec {
        #[arg(long)]
        version: Option<String>,
        #[arg(last = true, required = true)]
        arguments: Vec<OsString>,
    },
    /// Rescan and atomically publish command entries. / 重新扫描并原子发布命令入口。
    Rehash {
        version: Option<String>,
        #[arg(long, conflicts_with = "dry_run")]
        check: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Inspect runtime and shell configuration. / 检查运行时与终端配置。
    Doctor,
    /// Inspect interrupted work or discard one owned staging attempt. / 检查中断工作或丢弃一次拥有所有权的 staging 尝试。
    Recover {
        #[arg(long)]
        discard_staging: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Emit `PowerShell` completion without editing a profile. / 输出 `PowerShell` 补全脚本，不修改配置文件。
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Read or update trusted user configuration. / 读取或更新可信用户配置。
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    #[command(name = "__dispatch", hide = true)]
    Dispatch {
        filename: String,
        #[arg(last = true)]
        arguments: Vec<OsString>,
    },
    #[command(name = "__system-path", hide = true)]
    SystemPath {
        #[arg(long)]
        owner_sid: String,
        #[arg(long)]
        remove: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigAction {
    Get { key: Option<String> },
    Set { key: String, value: String },
    Unset { key: String },
}

#[derive(Subcommand)]
pub(crate) enum CacheAction {
    List,
    Remove {
        filename: String,
        #[arg(long)]
        yes: bool,
    },
}
