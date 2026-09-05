<p align="right">
  <a href="./CONTRIBUTING.md">简体中文</a> |
  <strong>English</strong>
</p>

# Contributing

Issues, documentation improvements, and code contributions are welcome. Discuss substantial features in an issue first to agree on scope and compatibility. For usage, see the [README](./README_EN.md).

## Branches and merge directions

| Branch | Purpose | Starting point and merge destination |
| --- | --- | --- |
| `master` | Reviewed release baseline; recommended default branch | Receives the corresponding `release/*` after release verification |
| `develop` | Day-to-day integration and the target for ordinary PRs | Receives feature, fix, and documentation PRs |
| `feature/*` | New functionality | Branch from and open a PR into `develop` |
| `fix/*` | Routine fixes | Branch from and open a PR into `develop` |
| `docs/*` | Documentation improvements | Branch from and open a PR into `develop` |
| `hotfix/*` | Urgent fixes to the release baseline | Branch from `master`, publish a new version, and synchronize fixes into `develop` |
| `release/<version>` | Public release snapshot | Branch from an exact reviewed `develop` or hotfix commit; merge into `master` after release verification |

`master` is a release baseline, not a promise of stability: it may represent an `alpha`, `beta`, or `rc` prerelease. Start ordinary contributions from `develop`, not a release branch.

Automation currently handles only `release/*`: pushing one of these branches runs checks, builds packages, and automatically publishes to GitHub Releases. This workflow does not run on `master`, `develop`, feature branches, or PRs. Complete local checks before opening a PR.

## Your first pull request

1. Fork this repository to your account, then clone your fork. Replace `<your-account>` below with your GitHub username.
2. Add the upstream repository and create a work branch from its latest `develop`.

~~~text
git clone https://github.com/<your-account>/PyRudder.git
cd PyRudder
git remote add upstream https://github.com/kuveil/PyRudder.git
git fetch upstream
git switch -c feature/runtime-list upstream/develop
~~~

3. Make a focused change, run the checks below, review your staged changes, and commit. For example:

~~~text
git add crates/pyrudder-cli
git diff --cached
git commit -m "feat(cli): improve runtime selection"
git push -u origin feature/runtime-list
~~~

4. Open a PR on GitHub with `kuveil/PyRudder` as the base repository and **`develop` as the base branch**. Describe the scope, user-visible behavior, related issue, and test results. Include redacted screenshots for visual or interactive changes when useful.

Replace the example branch, paths, and commit message with your actual change. Documentation work can use `docs/contribution-guide`; a bug fix can use `fix/runtime-selection`.

## Development and local checks

Use Windows 10/11 x64, PowerShell 7, Rust stable meeting the `rust-version` in `Cargo.toml`, and the MSVC C++ build tools with the Windows SDK. The Rust toolchain needs the `rustfmt`, `clippy`, and `rust-docs` components; the last supplies standard-library license notices for validation.

Run these basic checks from the repository root to match the release workflow. Address any failed command; a successful final command does not imply that earlier checks passed.

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
./scripts/tests/release-validation.ps1
./scripts/tests/release-publish-tests.ps1
./scripts/tests/package-validation.ps1
./scripts/tests/package-notices.Tests.ps1
~~~

Changes to the installer, PATH handling, version switching, or downloads also need relevant behavioral testing in an isolated environment. Do not run destructive tests against your everyday Python installations or system environment. State any checks you did not run, and why, in the PR.

For installer upgrade regression testing, run `scripts/tests/installer-upgrade.Tests.ps1` with explicit `-OldPayload`, `-NewPayload` (complete unpacked directories with different binaries), `-InnoCompiler`, and `-PythonDirectory` arguments. The old version must be at least `0.1.0`, and the new version must not be older. The test uses a separate installation identity and disposable directories under `target/`, opts out of PATH changes, and invokes its test uninstaller afterward. It checks directory detection, data preservation, failure recovery, same-version repair, and downgrade rejection. Existing Python is used only for registration and command checks; its installation contents are not modified.

Use commit messages in the form `type(scope): feature or fix`, such as `fix(shim): preserve child exit codes` or `docs(readme): clarify installation options`. Keep each commit focused on one concern and avoid unrelated formatting or refactoring.

## Documentation, privacy, and licensing

- Maintain separate Chinese and English public documents with language links at the top. Update both usage guides when user-visible behavior changes.
- Write new or changed code comments in Chinese and English, explaining constraints, reasons, and behavior that is easy to misuse.
- Do not commit local requirements, plans, progress notes, history backups, or other development records under `local-docs/`. Do not commit build artifacts or test data under `target/`.
- Do not commit credentials, personal email addresses, local usernames, real installation paths, private diagnostic output, or sensitive screenshot content. Use generic placeholders in examples and inspect `git diff --cached` before committing.
- Use your own commit identity. To protect your email address, obtain your own `noreply` address from GitHub email settings and configure Git to use it.
- Contributions are provided under the project's [Apache License 2.0](./LICENSE). Preserve applicable licenses, copyright notices, and attribution when adding third-party content.

## Maintainers: releases and branch protection

1. Integrate ordinary features and fixes into `develop` through PRs. Before release, update the Cargo workspace version, relevant lockfile versions, and public documentation on the branch being prepared, then complete review and local verification.
2. Create `release/<version>` from the exact reviewed commit only when ready to publish: use `develop` for ordinary releases, or `hotfix/*` for an urgent release without unfinished features. The branch version must exactly match the Cargo workspace version, for example `release/0.1.0`; a subsequent patch could use `release/0.1.1`. **Pushing triggers public release automatically, without a separate publication approval.**
3. After successful validation, the workflow creates the `v<version>` tag and Release, marking versions with prerelease identifiers as prereleases. Verify the published artifacts and outcome, then merge the release branch into `master`.
4. Treat released branches and tags as read-only snapshots: do not append commits, rewrite them, or force-push. A version cannot be reused for another commit. Return subsequent fixes to `develop`, increment the version, and create a new release branch. Keep historical prerelease branches and tags unchanged as well.
5. Start urgent fixes from `master` on `hotfix/*`, review them, and publish a new version through the process above. After release verification, merge into `master` and synchronize the fixes into `develop` through a PR, preserving any newer version already planned on the development line. Do not leave fixes only on `master` or push them directly to an old release branch.

Maintainers should set `master` as the default branch in GitHub repository settings, require PRs for `master` and `develop`, and prevent force-pushes and deletion. Restrict changes to release branches and tags. With `master` as the default, contributors must still select `develop` for ordinary PRs.

A sole maintainer cannot approve their own PR. Initially use zero required approvals or a controlled administrator bypass; require at least one approval once a second trusted maintainer is available, so maintainers do not lock themselves out of merging their own changes.

These are configuration recommendations, not a claim that remote protection is already enabled. PRs currently have no automated checks; do not require release-only status checks for ordinary PRs.
