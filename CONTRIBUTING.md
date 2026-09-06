<p align="right">
  <strong>简体中文</strong> |
  <a href="./CONTRIBUTING_EN.md">English</a>
</p>

# 参与贡献

欢迎提交问题、改进文档或贡献代码。较大功能请先通过 Issue 讨论目标和兼容性；使用方法见 [README](./README_ZH.md)。

## 分支与合并方向

| 分支 | 用途 | 从哪里创建、合并到哪里 |
| --- | --- | --- |
| `master` | 已审核的发布基线、建议的仓库默认分支 | 发布验证通过后接收对应 `release/*` 的合并 |
| `develop` | 日常开发集成、普通 PR 的目标 | 接收功能、修复和文档 PR |
| `feature/*` | 新功能 | 从 `develop` 创建，PR 回 `develop` |
| `fix/*` | 常规修复 | 从 `develop` 创建，PR 回 `develop` |
| `docs/*` | 文档改进 | 从 `develop` 创建，PR 回 `develop` |
| `hotfix/*` | 发布基线的紧急修复 | 从 `master` 创建，通过新版本发布，修复同步回 `develop` |
| `release/<版本号>` | 公开发布快照 | 从已审核的 `develop` 或紧急修复提交创建，发布验证后合并回 `master` |

`master` 表示发布基线，不等于稳定版；它可能对应 `alpha`、`beta` 或 `rc` 预发布。日常贡献请以 `develop` 为基线，不要直接修改发布分支。

当前自动化只处理 `release/*`：推送该类分支会执行检查、打包并自动发布到 GitHub Releases。`master`、`develop`、功能分支和 PR 不运行此流水线；提交 PR 前必须完成本地检查。

## 提交你的第一个 PR

1. Fork 本仓库到自己的账户，然后克隆自己的 Fork。将下方 `<你的账户>` 替换为实际 GitHub 用户名。
2. 添加上游仓库，并从最新的上游 `develop` 创建工作分支。

~~~text
git clone https://github.com/<你的账户>/PyRudder.git
cd PyRudder
git remote add upstream https://github.com/kuveil/PyRudder.git
git fetch upstream
git switch -c feature/runtime-list upstream/develop
~~~

3. 完成范围明确的修改，运行下节检查，审查暂存内容后提交。例如：

~~~text
git add crates/pyrudder-cli
git diff --cached
git commit -m "feat(cli): improve runtime selection"
git push -u origin feature/runtime-list
~~~

4. 在 GitHub 创建 PR：基础仓库为 `kuveil/PyRudder`，**目标分支选择 `develop`**。说明修改范围、用户可见行为、关联 Issue 和测试结果；界面或交互变化可附脱敏截图。

示例中的分支、文件和提交说明应替换为本次实际修改。文档修改可用 `docs/contribution-guide`；错误修复可用 `fix/runtime-selection`。

## 开发与本地检查

开发环境：Windows 10/11 x64、PowerShell 7、Rust stable（至少符合 `Cargo.toml` 中的 `rust-version`），以及 MSVC C++ 构建工具和 Windows SDK。Rust 工具链需要 `rustfmt`、`clippy`、`rust-docs` 组件；最后一项用于校验标准库许可声明。

在仓库根目录运行与发布流水线一致的基本检查；任一命令失败时先修复，不要仅根据最后一条命令判断结果：

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
./scripts/tests/release-validation.ps1
./scripts/tests/release-publish-tests.ps1
./scripts/tests/package-validation.ps1
./scripts/tests/package-notices.Tests.ps1
~~~

修改安装器、PATH、版本切换或下载逻辑时，还应在隔离的测试环境中验证相关行为。不要对日常使用的 Python 或系统环境执行破坏性测试。请在 PR 中注明未执行的验证及原因。

安装器更新回归可运行 `scripts/tests/installer-upgrade.Tests.ps1`，显式传入 `-OldPayload`、`-NewPayload`（两份不同程序的完整解包目录）、`-InnoCompiler` 和 `-PythonDirectory`。旧版至少为 `0.1.0`，新版不低于旧版；测试使用独立安装标识和 `target/` 内的临时目录，取消 PATH 配置，并在结束后调用测试卸载器。它验证自动沿用目录、保留数据、失败恢复、同版修复及降级拒绝；已有 Python 仅用于登记和命令运行检查，不修改其安装内容。

提交说明采用 `类型(范围): 功能或修复说明`，例如 `fix(shim): preserve child exit codes`、`docs(readme): clarify installation options`。每次提交尽量只解决一个问题，避免夹带无关格式化或重构。

## 文档、隐私与许可证

- 对外文档的中文、英文分别维护，并在顶部提供语言切换；默认英文 README 为 `README.md`，中文版为 `README_ZH.md`；修改用户行为时同步更新对应教程。
- 新增或修改的代码注释使用中英文，重点解释约束、原因和容易误用的行为。
- 不提交 `local-docs/` 中的本地需求、计划、进度、历史备份或其他开发记录；不提交 `target/` 下的产物和测试数据。
- 不提交凭据、个人邮箱、本机用户名、真实安装路径、私有诊断输出或截图中的敏感信息。示例使用通用占位值；提交前检查 `git diff --cached`。
- 使用自己的提交身份；需要保护邮箱时，在 GitHub 邮箱设置中获取自己的 `noreply` 地址并配置 Git。
- 贡献按项目的 [Apache License 2.0](./LICENSE) 提供；引入第三方内容时保留原许可证、版权及必要声明。

## 维护者：发布与分支保护

1. 普通功能与修复先通过 PR 集成到 `develop`。发布前在准备发布的分支更新 Cargo 工作区版本、锁文件中相关版本及对外文档，并完成审核和本地验证。
2. 仅在准备公开发布时，从已审核的确切提交创建 `release/<版本号>`：普通发布取自 `develop`，紧急发布可取自 `hotfix/*`，避免夹带未完成的新功能。分支中的版本号必须与 Cargo 工作区版本完全一致，例如 `release/0.1.0`；后续修复可提升为 `release/0.1.1`。**推送即触发公开发布，没有额外的人工发布确认。**
3. 流水线验证通过后自动创建 `v<版本号>` 标签和 Release；带预发布标识的版本发布为 prerelease。确认下载产物及发布结果可用后，将该发布分支合并回 `master`。
4. 发布后的分支和标签视为只读快照，不追加提交、不改写、不强制推送。相同版本号不能用于其他提交；后续修复回到 `develop`，提升版本号后创建新的发布分支。历史预发布分支与标签也保持原样。
5. 紧急修复从 `master` 创建 `hotfix/*`，完成审核后按上述流程发布新的版本。发布验证后合并回 `master`，并将修复通过 PR 同步回 `develop`，保留开发线已经规划的较新版本号。不要仅修 `master` 而让后续开发丢失修复，也不要直接向旧发布分支推送。

建议维护者在 GitHub 仓库设置中将 `master` 设为默认分支，并为 `master`、`develop` 要求 PR 合并、禁止强制推送和删除；发布分支与标签应限制修改。默认分支设为 `master` 后，提交普通 PR 时仍需手动选择 `develop`。

单维护者不能批准自己的 PR，可先将必需批准人数设为 0，或保留受控的管理员例外；有第二位可信维护者后，再要求至少 1 人批准，避免将维护者自己的修改锁死。

这些是维护配置建议，文档不代表远程保护已经启用。当前 PR 没有自动检查，不要把仅发布时才运行的检查设置成普通 PR 的必需状态检查。
