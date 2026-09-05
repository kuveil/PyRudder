<p align="right">
  <a href="./README.md">简体中文</a> |
  <strong>English</strong>
</p>

<p align="center">
  <img src="assets/pyrudder-logo.svg" alt="PyRudder" width="180">
</p>

<h1 align="center">PyRudder</h1>

<p align="center">Multiple runtimes. Ordinary commands. Your course.</p>

PyRudder is a Python version manager for Windows. Register existing Python installations or download official Python, select a version with `pyrudder`, and keep using ordinary commands such as `python` and `pip`.

Source version: `0.1.0` for Windows 10/11 x64; see [GitHub Releases](https://github.com/kuveil/PyRudder/releases) for downloadable versions. Windows installers are not yet code-signed. After installation, PyRudder is CLI-only: no desktop app, tray, background service, or shell initialization script.

## 1. Install PyRudder

Download the installer or ZIP for your version from [GitHub Releases](https://github.com/kuveil/PyRudder/releases). The accompanying `.sha256` files let you check download integrity. PyRudder itself does not bundle Python.

### Installer (recommended)

For a first installation, open `pyrudder-…-Setup.exe`, choose the interface language, and select a new empty directory, such as `D:\Tools\PyRudder`. See "Update an existing installation" below for updates from `0.1.0` onward; Alpha versions require a fresh installation as described there.

“Add PyRudder to system PATH” is checked by default and can be cleared. When selected, setup requests Administrator permission, appends `bin` and `shims` to system PATH, and removes only the installing account's `WindowsApps` entry from system PATH. It does not modify user PATH or delete Store apps or the WindowsApps directory. When cleared, PATH is unchanged and the finish page shows the two directories to configure manually.

System PATH affects every local user and Administrator terminal, while the installation directory remains controlled by the installing account. This creates cross-user command-resolution risks; enable this option only on a trusted single-user development machine. If automatic configuration fails or elevation is cancelled, setup reports an incomplete installation. You do not need to uninstall first: run the installer again in the same directory, or retry PATH configuration with `pyrudder path add`.

After installation, fully close and reopen your terminal, including hosts such as Windows Terminal or VS Code, then run:

~~~text
pyrudder --version
~~~

Without PATH configuration, use the full path to `pyrudder.exe` inside `bin`. You can later run `pyrudder path add` to apply the system PATH configuration above with Administrator permission.

### Update an existing installation

`0.1.0` establishes a new installation baseline and does not take over Alpha installations. If you used `0.1.0-alpha.8` or another Alpha version, back up any data you need, uninstall the old program, install `0.1.0` into a new empty directory, and register existing Python again. Directories retained after uninstall are not taken over automatically.

Starting with `0.1.0`, the installer supports updates in the original directory under the same Windows account. Running the same installer version again can also repair an installation. When a newer version is released:

1. Download the newer `pyrudder-…-Setup.exe` from [GitHub Releases](https://github.com/kuveil/PyRudder/releases). Do not uninstall the old version first.
2. Finish running PyRudder commands and stop Python, pip, and other processes launched through PyRudder. Close related terminals, editors, and Jupyter sessions when needed.
3. Run the new installer as the original installing account. The wizard detects the previous installation directory and updates it in place, without asking you to locate or enter it again. Choosing another installation directory during an update is not supported.
4. Confirm the PATH option and complete setup. The wizard defaults to the selection from the previous installation. Clearing the option leaves the current PATH unchanged; it does not remove existing PATH entries.
5. Fully close and reopen your terminal, then check the version, registrations, and selection:

~~~text
pyrudder --version
pyrudder list
pyrudder current --explain
python --version
~~~

The update replaces PyRudder programs and refreshes command entry points while retaining configuration, Python registrations, aliases, the global selection, caches, managed Python, and custom runtime and download directories. Existing Python is not downloaded again; external Python and project selection files are not deleted. System PATH management records are preserved so a later uninstall can still reverse changes managed by this installation. After closing an old terminal, set any temporary `pyrudder shell` selection again in the new terminal if needed.

Updates back up programs and command entry-point information and automatically attempt to restore them on failure. If files are reported as in use, close the related processes before retrying. You should still back up important data before updating.

The installer rejects downgrades. Updating cannot migrate the installation directory, create multiple installer-managed copies, or take over another account's installation. If a complete supported installation cannot be identified, back up your data and check the installation state instead of manually overwriting files. Updates require downloading and running the new installer; PyRudder does not automatically check for or download updates. Portable ZIP overwrite upgrades are not supported.

### Portable ZIP

Extract into an empty directory you intend to keep, preserving the complete structure. Open a terminal there and run once:

~~~text
.\bin\pyrudder.exe setup --add-to-path
~~~

This initializes in place and requests Administrator permission to configure system PATH, with the same effect as the installer's PATH option. Omit `--add-to-path` to initialize without changing PATH. Reopen your terminal afterward. Installer users do not need this step. Moving an initialized directory is not currently supported.

## 2. Add Python

### Register an existing installation

Replace the example with an existing Python directory you trust, or the path to its `python.exe`:

~~~text
pyrudder register "D:\PythonVersions\Python313" --alias work
pyrudder list
~~~

Registration executes the specified Python to confirm its version, but does not move, copy, or delete it. `work` is a custom alias you can use instead of a version number. `list` shows one row per Python; use `pyrudder info work` for details.

### Download and install official Python

~~~text
pyrudder available
~~~

Use Up/Down to choose a version, then Enter to provide an installation parent directory. Each version uses its own subdirectory. An empty input always uses `runtimes` under the current PyRudder directory, such as `D:\Tools\PyRudder\runtimes`. A custom destination applies only to this installation; it does not change the default configuration or select the new Python automatically.

Press Esc or `q` to cancel selection, or Ctrl+C to cancel directory input. Selecting an installed version only shows its existing path; interrupted installations resume in their original directory. Downloads show progress, speed, and estimated time remaining. When total size is unknown, they show transferred bytes and speed. Downloads are verified before use, including reused cached files.

For a list without installation, use `pyrudder available --list`; for scripts, use `pyrudder available --json`. Redirected input or output and `--quiet` also disable interactive installation. You can instead specify a version and alias directly:

~~~text
pyrudder install 3.14 --alias py314
~~~

`available`, `download`, and `install` support `--offline`, using only cached indexes and archives and reporting missing cache explicitly. Official indexes undergo signature verification, and downloaded archives are checked against SHA-256 hashes.

## 3. Select a Python version

For everyday use, set a global default and run Python normally. This example uses the registered alias `work`; after installing through `available`, use a version shown by `pyrudder list`, such as `3.14`, instead:

~~~text
pyrudder global work
python --version
pip --version
~~~

The following are independent options for different needs, not steps to run in sequence:

| Need | Command | Scope |
| --- | --- | --- |
| Set a default | `pyrudder global work` | Used when no higher-priority selection exists |
| Pin a project | `pyrudder local py314` | Run in the project directory; applies there and below |
| Switch temporarily | `pyrudder shell py314` | This terminal and its children, not independently opened terminals |
| Select for one command | `pyrudder exec --version py314 -- python --version` | Only this invocation |

Precedence is explicit `exec --version` > current terminal > nearest project selection > global default. Aliases and short versions are pinned to the installation selected at that time; installing a newer version does not update the selection automatically. An activated virtual environment takes precedence for its own commands.

Use `pyrudder current --explain` to inspect the current choice and its source. Clear a selection with the corresponding `pyrudder global --unset`, `pyrudder local --unset`, or `pyrudder shell --unset`. Run the project command in that project's directory. `shell --unset` also stops inheriting a parent terminal's session selection.

## 4. Choose installation and download directories

Before the first managed installation, you can configure defaults for direct `pyrudder install` commands:

~~~text
pyrudder config set paths.runtimes_dir "D:\PythonVersions"
pyrudder config set paths.downloads_dir "D:\PythonDownloads"
pyrudder config get
~~~

`runtimes_dir` stores installed Python; `downloads_dir` stores downloaded archives only. Use absolute local paths that do not contain or overlap other configured directories. You cannot change the default runtime directory while managed Python records exist. Configuration changes do not migrate existing Python.

For a single direct installation elsewhere, use `pyrudder --runtimes-dir "D:\OtherPythonVersions" install 3.13 --alias py313`. Empty input in interactive `available` always uses `runtimes` under PyRudder, not this configured default.

## 5. Remove Python or uninstall PyRudder

First switch or clear any global and terminal selections referencing that Python, then choose the operation for its source:

| Operation | Command | Deletes Python files? |
| --- | --- | --- |
| Unregister external Python | `pyrudder unregister work --yes` | No; the existing installation is preserved |
| Uninstall Python downloaded by PyRudder | `pyrudder uninstall py314 --yes` | Yes, including its installed packages; cannot be undone |

Managed uninstall uses the registered destination without requiring its custom path again. Project selection files are not deleted automatically. Select another version or run `pyrudder local --unset` in affected projects.

Installer users can remove PyRudder through Windows Installed apps. Uninstall reverses this installation's managed system PATH changes, restores its removed WindowsApps entry, and removes the CLI. It requests Administrator permission when needed; failure preserves the CLI for retry. Configuration, registrations, shims, caches, and managed Python are retained. External Python is never deleted. Retained shims cannot work without the CLI. To delete managed Python, run the `uninstall` command above before removing the CLI.

ZIP users should run `pyrudder path remove` to reverse this installation's PATH changes, then handle the extracted directory and any data they want to retain. Remove manually configured PATH entries yourself.

For routine updates from `0.1.0` onward, run the newer installer without uninstalling first. Alpha versions require a backup and fresh installation as described above. Data retained after uninstall is not a complete installed application. To redeploy or change the directory, back up any data you need, uninstall the old program, install into a new empty directory, and register existing Python again. Automatic migration and multiple installer-managed copies are not supported.

## FAQ and supported scope

- **Does `python` still open the Store or run another version?** Fully exit and reopen the terminal host, and confirm setup did not report a PATH failure. PyRudder appends system PATH; it does not guarantee precedence over every existing system Python, terminal alias, or virtual environment. Use `pyrudder doctor` for diagnostics and `pyrudder which python` for the selected target. `pyrudder exec -- python --version` explicitly runs through PyRudder. Do not delete the WindowsApps directory.
- **Is a version or command missing?** Check registrations with `pyrudder list` and selection with `pyrudder current --explain`. Missing commands in the selected version do not fall back to another Python. Run `pyrudder rehash` if newly installed commands are not visible.
- **Was a download or installation interrupted?** Retry the original command. `pyrudder recover` reports unfinished operations. Check what data is needed before cleanup; do not manually delete a runtime in use.
- Supported targets are Windows 10/11 x64, PowerShell/CMD, and stable standard CPython x64. PyRudder does not take over `py.exe`. System Python fallback, ARM64/x86, Git Bash, PyPy, prerelease or free-threaded Python, network/reparse-point directories, online self-update, ZIP overwrite upgrades, and installation-directory migration are not supported. Run only Python installations and scripts you trust.

See `pyrudder --help` or `pyrudder <command> --help` for more options. Before reporting an issue, review diagnostics and remove personal paths, account information, and other sensitive data.

## Contributing

Issues, documentation improvements, and code contributions are welcome. See the [contribution guide](https://github.com/kuveil/PyRudder/blob/master/CONTRIBUTING_EN.md) for branches, local checks, and pull requests. Start everyday work from `develop` and target ordinary PRs at `develop`.

## License

PyRudder uses the [Apache License 2.0](LICENSE). Bundled third-party dependencies retain their respective licenses.
