## 变更摘要 / Summary

说明本次修改的范围、功能及原因。关联 Issue（如有）。
Describe the scope, behavior changes, and motivation. Link related issues if any.

## 验证 / Verification

列出实际执行的检查及结果；未执行或失败的检查请明确说明。
List checks actually run and their results; disclose checks that were skipped or failed.

## 检查清单 / Checklist

- [ ] 普通贡献以 `develop` 为目标，而不是 `master` 或 `release/*`。 / Routine contributions target `develop`, not `master` or `release/*`.
- [ ] 修改范围明确；没有无关改动。 / The scope is clear and contains no unrelated changes.
- [ ] 已执行适用检查；当前普通 PR 不自动运行 CI。 / Applicable checks are documented; routine PRs do not currently run CI automatically.
- [ ] 用户行为变更已同步中英文 README；新增代码注释使用中英文。 / User-facing changes update both READMEs; new code comments use Chinese and English.
- [ ] 不含密钥、私人邮箱、本机路径、个人测试数据或本地开发记录。 / No secrets, private email addresses, machine-specific paths, personal test data, or local development records are included.
- [ ] PATH、权限、文件删除、下载或兼容性变化已明确说明。 / Changes to PATH, permissions, file deletion, downloads, or compatibility are explained.

维护者发布同步 PR 可以目标为 `master`，请说明发布标签、对应提交及验证结果。
Maintainer release-sync PRs may target `master`; identify the release tag, source commit, and verification results.
