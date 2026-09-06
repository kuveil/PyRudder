<p align="right">
  <a href="./README.md">English</a> |
  <strong>简体中文</strong>
</p>

<p align="center">
  <img src="assets/pyrudder-logo.svg" alt="PyRudder" width="180">
</p>

<h1 align="center">PyRudder</h1>

<p align="center">多个运行时，一个命令，由你掌舵。</p>

PyRudder 是 Windows 上的 Python 多版本管理器。登记已有 Python 或下载官方 Python 后，使用 `pyrudder` 切换版本，照常运行 `python`、`pip` 等命令。

源码版本：`0.1.0`，适用于 Windows 10/11 x64；可下载版本以 [GitHub Releases](https://github.com/kuveil/PyRudder/releases) 为准。Windows 安装包尚未进行代码签名。安装后只有 CLI，没有桌面应用、托盘程序或常驻服务，也不需要终端初始化脚本。

## 1. 安装 PyRudder

从 [GitHub Releases](https://github.com/kuveil/PyRudder/releases) 下载与版本对应的安装包或 ZIP；同页的 `.sha256` 文件可用于核对下载文件的完整性。PyRudder 本身不捆绑 Python。

### 安装包（推荐）

首次安装时，打开 `pyrudder-…-Setup.exe`，选择界面语言和新的空目录，例如 `D:\Tools\PyRudder`。`0.1.0` 及以后版本的更新方式见下方“更新已有安装”；Alpha 版本需按该节说明重新安装。

向导默认勾选“将 PyRudder 加入系统 PATH”，可以取消。勾选后会请求管理员授权，将 `bin`、`shims` 追加到系统 PATH，并仅移除系统 PATH 中当前安装账户对应的 `WindowsApps` 项；不修改用户 PATH、不删除商店应用或 WindowsApps 目录。取消勾选时不修改 PATH，完成页会提示需要自行配置的两个目录。

系统 PATH 对本机所有用户及管理员终端生效，而安装目录仍由安装账户管理。这会带来跨用户命令解析风险；请仅在受信任的单用户开发机上启用此选项。自动配置失败或取消授权时，安装器会明确报告安装未完整成功；无需先卸载，可以在原目录重新运行安装器，或使用 `pyrudder path add` 重试 PATH 配置。

安装完成后，完全关闭并重新打开终端（包括 Windows Terminal 或 VS Code 等终端宿主），然后运行：

~~~text
pyrudder --version
~~~

未配置 PATH 时，请使用 `bin` 中 `pyrudder.exe` 的完整路径。也可之后执行 `pyrudder path add`，经管理员授权完成上述系统 PATH 配置。

### 更新已有安装

`0.1.0` 是新的安装基线，不接管 Alpha 系列安装。使用过 `0.1.0-alpha.8` 等 Alpha 版本时，请先备份需要保留的数据，再卸载旧程序，选择新的空目录安装 `0.1.0` 并重新登记已有 Python。卸载后残留的旧目录不会被自动接管。

从 `0.1.0` 开始，安装包支持同一 Windows 账户下的原目录更新，也可重新运行相同版本的安装包进行修复。以后发布新版时，操作步骤如下：

1. 从 [GitHub Releases](https://github.com/kuveil/PyRudder/releases) 下载新版 `pyrudder-…-Setup.exe`，不必先卸载旧版。
2. 结束正在运行的 PyRudder 命令，以及通过 PyRudder 启动的 Python、pip 等进程；需要时关闭相关终端、编辑器与 Jupyter 会话。
3. 使用原安装账户运行新版安装包。向导自动识别旧安装目录并在原目录更新，不需要重新寻找或输入路径。更新期间不能改选其他安装目录。
4. 确认 PATH 选项后完成安装。向导默认沿用上次安装时的选择；取消勾选只是不修改当前 PATH，不会删除已有 PATH 项。
5. 完全关闭并重新打开终端，确认版本、登记和当前选择：

~~~text
pyrudder --version
pyrudder list
pyrudder current --explain
python --version
~~~

更新会替换 PyRudder 程序并刷新命令入口，保留配置、Python 登记、别名、全局选择、缓存、托管 Python，以及自定义运行时和下载目录。已有 Python 不会被重新下载，外部 Python 和项目版本配置也不会被删除。原有系统 PATH 管理记录继续保留，之后卸载仍可撤销由本安装管理的 PATH 修改。关闭旧终端后，其临时 `pyrudder shell` 选择需在新终端按需重新设置。

更新会备份程序与命令入口信息，失败时自动尝试恢复。若提示文件正在使用，请关闭相关进程后再试；重要数据仍建议在更新前自行备份。

安装器会拒绝降级，不支持借更新迁移安装目录、多份安装包并存或接管其他账户的安装。未识别到完整的受支持安装时，请先备份数据并确认安装状态，不要手动覆盖文件。本功能是手动下载新版安装包后更新，不是联网自动检查或下载更新；ZIP 便携包暂不支持覆盖升级。

### ZIP 便携包

解压到准备长期使用的空目录，保留完整目录结构。在该目录打开终端，执行一次：

~~~text
.\bin\pyrudder.exe setup --add-to-path
~~~

这会就地初始化并请求管理员授权配置系统 PATH，与安装包的 PATH 选项作用相同。省略 `--add-to-path` 可仅初始化、不修改 PATH。之后重新打开终端。安装包用户无需执行这一步；初始化后暂不支持直接移动目录。

## 2. 添加 Python

### 登记已经安装的 Python

将示例路径替换为你已有且信任的 Python 安装目录，也可以指向其中的 `python.exe`：

~~~text
pyrudder register "D:\PythonVersions\Python313" --alias work
pyrudder list
~~~

登记会执行指定的 Python 以确认版本，但不会搬动、复制或删除它。`work` 是自定义别名，后续可用它代替版本号。`list` 每份 Python 显示一行；详细信息使用 `pyrudder info work`。

### 下载并安装官方 Python

~~~text
pyrudder available
~~~

用上下键选版本，回车后输入本次安装的父目录；每个版本使用独立子目录。留空回车固定安装到当前 PyRudder 目录下的 `runtimes`，例如 `D:\Tools\PyRudder\runtimes`。自定义目录仅对本次安装生效，不更改默认配置，也不会自动切换当前 Python。

列表中按 Esc 或 `q` 取消，输入目录时按 Ctrl+C 取消。选中已安装版本只显示原路径；未完成的安装会继续使用原目录。下载时显示进度、速度和剩余时间；未知总大小时显示已下载大小和速度。下载完成后会校验文件，复用缓存时也会验证。

只想查看列表时用 `pyrudder available --list`；脚本读取用 `pyrudder available --json`。输入或输出重定向、`--quiet` 模式也不会触发交互安装。也可直接指定版本和别名：

~~~text
pyrudder install 3.14 --alias py314
~~~

`available`、`download`、`install` 支持 `--offline`，仅使用已缓存的索引与下载文件，缺少缓存时明确报错。官方索引经过签名验证，下载文件经过 SHA-256 校验。

## 3. 选择要使用的版本

通常只需设置全局默认版本，然后正常使用 Python。下例使用已登记的别名 `work`；若通过 `available` 安装，可改用 `pyrudder list` 中的版本号，例如 `3.14`：

~~~text
pyrudder global work
python --version
pip --version
~~~

下面是不同需求的独立选项，不需要依次执行：

| 需求 | 命令 | 作用范围 |
| --- | --- | --- |
| 设置默认版本 | `pyrudder global work` | 没有更高优先级选择时使用 |
| 固定某个项目 | `pyrudder local py314` | 在项目目录执行，对该目录及子目录生效 |
| 临时切换当前终端 | `pyrudder shell py314` | 当前终端及其子进程，不影响独立新开的终端 |
| 只为一次命令指定版本 | `pyrudder exec --version py314 -- python --version` | 仅此次命令 |

优先级为：单次 `exec --version` > 当前终端 > 最近的项目配置 > 全局默认。别名和短版本会固定到当时选中的具体安装；安装新版本不会自动更新选择。已激活的虚拟环境优先使用自身命令。

`pyrudder current --explain` 可查看当前选择及来源。用对应的 `pyrudder global --unset`、`pyrudder local --unset` 或 `pyrudder shell --unset` 取消选择；项目取消命令应在该项目目录执行。`shell --unset` 同时停止继承父终端的会话选择。

## 4. 控制安装与下载目录

直接使用 `pyrudder install` 时，可在首次托管安装前设置默认目录：

~~~text
pyrudder config set paths.runtimes_dir "D:\PythonVersions"
pyrudder config set paths.downloads_dir "D:\PythonDownloads"
pyrudder config get
~~~

`runtimes_dir` 保存安装后的 Python，`downloads_dir` 只保存下载文件。目录必须是本地绝对路径，且与其他配置目录互不包含。已有托管 Python 记录时不能修改默认运行时目录；修改配置不会迁移已有 Python。

单次直接安装到其他目录可用 `pyrudder --runtimes-dir "D:\OtherPythonVersions" install 3.13 --alias py313`。交互式 `available` 留空回车始终使用 PyRudder 目录下的 `runtimes`，不使用这里配置的默认值。

## 5. 移除 Python 或卸载 PyRudder

先切换或取消该 Python 的全局与终端选择，再按来源选择操作：

| 操作 | 命令 | 是否删除 Python 文件 |
| --- | --- | --- |
| 取消外部 Python 登记 | `pyrudder unregister work --yes` | 否，已有 Python 保留 |
| 卸载 PyRudder 下载的 Python | `pyrudder uninstall py314 --yes` | 是，包括该 Python 中安装的包，无法撤销 |

卸载托管 Python 会使用登记的实际目录，不必重复输入自定义安装路径。项目版本配置不会自动删除，请在相关项目中改选版本或执行 `pyrudder local --unset`。

安装包用户从 Windows“已安装的应用”卸载 PyRudder。卸载器会撤销本安装管理的系统 PATH 修改、恢复本安装移除的 WindowsApps 项，并删除 CLI；需要时请求管理员授权，失败则保留 CLI 以便重试。配置、登记、shim、缓存和托管 Python 均保留，外部 Python 不会删除。保留的 shim 不能脱离 CLI 使用。若要删除托管 Python，请在卸载 CLI 前执行上面的 `uninstall`。

ZIP 用户先执行 `pyrudder path remove` 撤销本安装的 PATH 修改，再自行处理解压目录和需要保留的数据。手动配置的 PATH 项需自行移除。

`0.1.0` 及以后版本的日常更新请使用新版安装包，不要先卸载。Alpha 系列需按上方说明备份后重新安装。卸载后的保留数据不等于完整的已安装程序；如需重新部署或改变目录，请先备份需要保留的数据、卸载旧程序，再选择新的空目录安装并重新登记已有 Python。自动迁移和多份安装包并存不在支持范围内。

## 常见问题与支持范围

- **`python` 仍打开商店或运行了其他版本？** 完全退出并重开终端宿主，确认安装器未报告 PATH 配置失败。PyRudder 追加系统 PATH，并不保证优先于所有已有系统 Python、终端别名或虚拟环境。用 `pyrudder doctor` 检查、`pyrudder which python` 查看选中目标；`pyrudder exec -- python --version` 可显式通过 PyRudder 运行。不要删除 WindowsApps 目录。
- **找不到版本或命令？** 先用 `pyrudder list` 查看登记状态，再用 `pyrudder current --explain` 检查选择。缺少所选版本的命令时不会自动回退到其他 Python；新装的命令未出现时可执行 `pyrudder rehash`。
- **下载或安装中断？** 重试原命令；`pyrudder recover` 可查看未完成操作。清理前确认数据用途，不要手动删除正在使用的运行时。
- 当前支持 Windows 10/11 x64、PowerShell/CMD 和稳定版标准 CPython x64；不接管 `py.exe`，不支持系统 Python 自动回退、ARM64/x86、Git Bash、PyPy、预发布或自由线程 Python。网络目录、重解析点目录、联网自更新、ZIP 覆盖升级和安装目录迁移不在支持范围内。运行时及其脚本必须来自你信任的来源。

更多参数见 `pyrudder --help` 或 `pyrudder <命令> --help`。提交问题前，请检查诊断内容并移除个人路径、账户信息及其他敏感数据。

## 参与贡献

欢迎提交问题、文档和代码改进。分支协作、本地检查及 PR 提交方式见[贡献指南](https://github.com/kuveil/PyRudder/blob/master/CONTRIBUTING.md)；日常开发以 `develop` 为基线，普通 PR 请提交到 `develop`。

## 许可证

PyRudder 使用 [Apache License 2.0](LICENSE)。随附的第三方依赖遵循各自的许可证。
