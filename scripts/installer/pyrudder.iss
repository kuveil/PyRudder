; One-time installation wizard; the installed product remains CLI-only.
; 一次性安装向导；安装后的产品仍然仅提供 CLI。
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef PackageVersion
  #error PackageVersion is required
#endif
#ifndef OutputName
  #error OutputName is required
#endif
#ifndef DistDir
  #error DistDir is required
#endif
; Test builds may isolate their uninstall identity; normal packages keep the historical ID.
; 测试构建可隔离卸载标识；正常发行包保持历史 ID 不变。
#ifndef AppIdentifier
  #define AppIdentifier "PyRudder.Alpha"
  #define InstallerMutex "PyRudderAlphaSetup"
#else
  #define InstallerMutex AppIdentifier + "Setup"
#endif
#ifdef InstallerTestFailBeforeCommit
  #if AppIdentifier == "PyRudder.Alpha"
    #error Fault injection requires an isolated test AppIdentifier
  #endif
#endif

[Setup]
AppId={#AppIdentifier}
AppName=PyRudder
AppVersion={#PackageVersion}
AppPublisher=PyRudder
AppPublisherURL=https://github.com/kuveil/PyRudder
DefaultDirName={localappdata}\PyRudder
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
MinVersion=10.0
DisableDirPage=no
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
UsePreviousTasks=yes
DirExistsWarning=no
DisableWelcomePage=no
WizardStyle=modern
OutputDir={#DistDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
ChangesEnvironment=yes
CloseApplications=no
RestartApplications=no
UninstallDisplayName=PyRudder (CLI)
SetupLogging=yes
SetupMutex={#InstallerMutex}

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Default.isl,ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Selected by default, but users and /TASKS may opt out of machine PATH changes.
; 默认勾选，用户及 /TASKS 参数均可取消系统 PATH 配置。
Name: "systempath"; Description: "{cm:SystemPathTask}"; GroupDescription: "{cm:SystemPathGroup}"

[Files]
; Run upgrade checks and rollback with the new trusted payload, never an old installed CLI.
; 使用安装包中的新程序执行升级检查与回滚，不执行旧安装中的 CLI。
Source: "{#PayloadDir}\bin\pyrudder.exe"; DestName: "pyrudder-installer-helper.exe"; Flags: dontcopy noencryption
Source: "{#PayloadDir}\bin\*.exe"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "{#PayloadDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\README_EN.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\BUILD-INFO.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\SBOM.cdx.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\SHA256SUMS.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion
; Initialize only after all program files exist; no Python is executed by setup.
; 所有程序文件就位后才初始化；安装不执行 Python。
Source: "{#PayloadDir}\bin\pyrudder-layout.json"; DestDir: "{app}\bin"; Flags: ignoreversion; AfterInstall: InitializeInstallation

[UninstallDelete]
; Only known generated program helpers, never recursive user-data removal.
; 仅清理已知的生成程序辅助文件，绝不递归删除用户数据。
Type: files; Name: "{app}\bin\pyrudder-location.json"
Type: dirifempty; Name: "{app}\bin"

[CustomMessages]
chinesesimplified.SystemPathTask=将 PyRudder 加入系统 PATH（推荐，需要管理员权限）
english.SystemPathTask=Add PyRudder to system PATH (recommended; Administrator permission required)
chinesesimplified.SystemPathGroup=可选命令行配置；不勾选时可在安装后自行配置环境变量：
english.SystemPathGroup=Optional command-line configuration; leave unchecked to configure PATH yourself:
chinesesimplified.ManualPathFinished=PyRudder CLI 已安装。你未选择自动配置系统 PATH，安装器没有修改 PATH。%n%n如需直接使用 pyrudder 和 python 命令，请自行将以下两个目录加入 PATH，然后完全关闭并重新打开终端：%n%1%n%2%n%n也可以使用完整路径运行 %3。无需初始化脚本。
english.ManualPathFinished=PyRudder CLI is installed. Automatic system PATH configuration was not selected, so setup did not modify PATH.%n%nTo use pyrudder and python directly, add these two directories to PATH yourself, then fully close and reopen your terminal:%n%1%n%2%n%nYou can also run %3 by its full path. No initialization script is needed.
chinesesimplified.EmptyDirectory=请选择一个新的空目录。未登记为受支持安装的目录不会被覆盖；ZIP 目录及卸载后保留的数据不会自动迁移。
english.EmptyDirectory=Choose a new empty directory. Unregistered directories are not overwritten; ZIP installations and data retained after uninstall are not migrated automatically.
chinesesimplified.InvalidPreviousInstallation=现有 PyRudder 安装记录无效或原目录已移动，无法安全更新。请先检查 Windows“已安装的应用”中的安装记录，不要删除配置或 Python 数据。
english.InvalidPreviousInstallation=The existing PyRudder installation record is invalid or its directory was moved. Check its Windows Installed apps entry before updating; do not delete configuration or Python data.
chinesesimplified.LegacyInstallation=检测到旧预发布版本 %1。0.1.0 是新的安装基线，不支持覆盖升级旧 alpha/beta/rc。请先卸载旧 CLI，再选择新的空目录安装；旧配置和 Python 数据会保留，但不会自动迁移。
english.LegacyInstallation=An older prerelease (%1) is installed. Version 0.1.0 starts a new installation baseline and does not upgrade old alpha/beta/rc builds in place. Uninstall the old CLI, then install into a new empty directory. Old configuration and Python data are preserved but are not migrated automatically.
chinesesimplified.UpgradeDirectory=检测到 PyRudder %1，将更新为 %2。安装目录保持不变。
english.UpgradeDirectory=PyRudder %1 will be updated to %2 in its existing directory.
chinesesimplified.UpgradeDirectoryLocked=更新使用原安装目录；不创建第二份安装，也不迁移数据。
english.UpgradeDirectoryLocked=Updates use the existing installation directory; a second installation is not created and data is not moved.
chinesesimplified.UpgradeDirectoryChanged=必须在原安装目录更新：%1。请不要通过 /DIR 或其他方式更改更新目录。
english.UpgradeDirectoryChanged=Updates must use the original installation directory: %1. Do not change the update directory using /DIR or other overrides.
chinesesimplified.UpgradeReady=原位更新：%1 → %2。保留现有配置、Python 登记及托管 Python。更新前请关闭 PyRudder 和通过它启动的程序。系统 PATH 沿用上次勾选；取消勾选只表示本次不修改 PATH，不会移除已有项。
english.UpgradeReady=In-place update: %1 to %2. Existing configuration, Python registrations and managed Python are preserved. Close PyRudder and programs launched through it before continuing. The previous system PATH choice is reused; clearing it skips PATH changes for this run and does not remove existing entries.
chinesesimplified.InstallerPhaseFailed=安装检查或更新步骤“%1”失败，退出码 %2。请关闭正在运行的 PyRudder/Python 后重试；版本过旧、降级或目录状态异常时请查看安装日志。%n%n%3
english.InstallerPhaseFailed=Installer check/update step "%1" failed with exit code %2. Close running PyRudder/Python processes and retry. An unsupported old version, downgrade or inconsistent directory requires checking the setup log.%n%n%3
chinesesimplified.UpgradeRollbackFailed=更新未完成，且自动恢复未完整成功。请保留原安装目录及其中的更新备份，查看安装日志；不要删除配置或 Python 数据。
english.UpgradeRollbackFailed=The update did not complete and automatic restoration could not finish. Keep the installation directory and its update backup, and check the setup log. Do not delete configuration or Python data.
chinesesimplified.UpgradeFailedHeading=PyRudder 更新未完成
english.UpgradeFailedHeading=PyRudder update did not complete
chinesesimplified.UpgradeFailedRecovered=更新未正常完成，已恢复原安装（版本 %1）。配置及 Python 数据已保留，本次未配置 PATH。请查看安装日志，并重新运行安装包。%n%n%2
english.UpgradeFailedRecovered=The update did not complete; the previous installation (%1) was restored. Configuration and Python data were preserved, and PATH was not configured. Check the setup log, then rerun the installer.%n%n%2
chinesesimplified.UpgradeFailedPreserved=更新完成步骤失败，已保留当前安装文件（版本 %1），没有回退已提交的新版本，本次未配置 PATH。请查看安装日志，并重新运行同版本安装包完成修复。%n%n%2
english.UpgradeFailedPreserved=The final update step failed. The current installation (%1) was retained without reverting a committed update, and PATH was not configured. Check the setup log, then rerun the same-version installer to repair it.%n%n%2
chinesesimplified.UpgradeManualPathFinished=PyRudder CLI 已在原目录更新。配置、Python 登记和托管 Python 已保留。%n%n本次未选择配置系统 PATH，现有 PATH 保持不变。请关闭并重新打开终端，使用 pyrudder --version 查看版本。
english.UpgradeManualPathFinished=PyRudder CLI was updated in its original directory. Configuration, Python registrations and managed Python were preserved.%n%nSystem PATH configuration was not selected for this run, so existing PATH entries were left unchanged. Close and reopen your terminal, then use pyrudder --version to check the version.
chinesesimplified.InvalidDirectory=请选择本地盘符下的普通目录（最多 160 字符），不要选择盘符根目录、链接目录，或含 %% ; ! 的目录。
english.InvalidDirectory=Choose an ordinary local drive directory (up to 160 characters), not a drive root, link, or path containing %% ; !.
chinesesimplified.SetupFailed=初始化失败，退出码 %1。请查看安装日志。请勿删除已有的 Python 或配置数据。
english.SetupFailed=Initialization failed with exit code %1. Check the setup log. Do not delete existing Python or configuration data.
chinesesimplified.PathRemoveFailed=无法恢复本安装清理的 WindowsApps 系统 PATH 项，或无法移除本安装添加的系统 PATH 项；CLI 文件尚未删除，卸载已中止。请重新运行卸载并完成管理员授权，详情见卸载日志。
english.PathRemoveFailed=Cannot restore the WindowsApps machine PATH entry cleaned by this installation or remove its owned machine PATH entries. CLI files were kept and uninstall stopped. Rerun uninstall and approve Administrator permission; see the uninstall log.
chinesesimplified.PathAddFailed=CLI 文件已安装或更新，但系统 PATH 配置或 WindowsApps 冲突项清理失败。安装未完整成功；请保留现有数据，查看安装日志，并重跑同版本安装包完成管理员授权。
english.PathAddFailed=CLI files were installed or updated, but machine PATH configuration or WindowsApps conflict cleanup failed. Installation is incomplete. Keep existing data, check the setup log, and rerun the same-version installer with Administrator permission.
chinesesimplified.DataPreserved=CLI 已卸载，本安装记录的 PATH 修改（如有）已撤销。配置、登记信息、shim 和托管 Python 数据保留在原目录；外部 Python 与商店应用文件未删除。
english.DataPreserved=The CLI was removed and any PATH changes recorded by this installation were undone. Configuration, registrations, shims and managed Python data remain in place; external Python and Store app files were not deleted.

[Messages]
chinesesimplified.WelcomeLabel2=本向导安装 PyRudder 命令行工具。首次安装可选择目录；0.1.0 及后续版本自动识别原目录更新，并保留配置及 Python 数据。旧预发布版本需先卸载，不能覆盖更新。%n%n“加入系统 PATH”首次默认勾选，更新时沿用上次选择，也可取消。勾选后请求管理员授权，将 bin 和 shims 追加到系统 PATH（对所有用户生效），仅移除系统 PATH 中当前账户的 WindowsApps 项。不会写入用户 PATH，也不删除商店应用。%n%n安装后无桌面应用、托盘或服务，不自动安装 Python。此安装包未签名。
english.WelcomeLabel2=This wizard installs the PyRudder CLI. Choose a directory for a new installation; version 0.1.0 and later update in the detected existing directory while preserving configuration and Python data. Older prereleases must be uninstalled first.%n%nAdd to system PATH is selected for new installs and reuses the previous choice for updates; it can be cleared. When selected, setup requests Administrator permission, appends bin and shims to machine PATH for all users, and removes only the current account's WindowsApps entry from machine PATH. User PATH and Store apps are unchanged.%n%nNo desktop app, tray or service; Python is not installed automatically. This installer is unsigned.
chinesesimplified.FinishedLabelNoIcons=PyRudder CLI 已安装并配置系统 PATH。请完全关闭并重开终端，直接使用 pyrudder 命令。%n%n登记已有 Python：%npyrudder register "D:\Python313" --alias work%npyrudder global work%n%n项目固定：pyrudder local work%n当前终端：pyrudder shell work%n%n安装器已自动处理当前账户的 WindowsApps 系统 PATH 冲突；不需要初始化脚本。
english.FinishedLabelNoIcons=PyRudder CLI is installed and machine PATH is configured. Fully close and reopen your terminal, then use pyrudder directly.%n%nRegister existing Python:%npyrudder register "D:\Python313" --alias work%npyrudder global work%n%nPin a project: pyrudder local work%nThis terminal: pyrudder shell work%n%nSetup automatically handled the current account's WindowsApps machine PATH conflict. No initialization script is needed.
chinesesimplified.ConfirmUninstall=卸载 PyRudder CLI？如果本安装曾修改系统 PATH，将请求管理员授权移除其添加的项，并恢复其清理的 WindowsApps 项；取消或失败时不删除 CLI。%n%n保留配置、登记信息、shim 和托管 Python；不删除外部 Python 或商店应用文件。
english.ConfirmUninstall=Remove the PyRudder CLI? If this installation changed system PATH, Administrator permission will be requested to remove its added entries and restore its cleaned WindowsApps entry. Cancellation or failure keeps the CLI files.%n%nConfiguration, registrations, shims and managed Python are preserved. External Python and Store app files are not deleted.

[Code]
const
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppIdentifier}_is1';

var
  PathFailed: Boolean;
  HasPreviousInstallation: Boolean;
  PreviousDirectory: String;
  PreviousVersion: String;
  UpgradePrepared: Boolean;
  UpgradeCommitted: Boolean;
  UpgradeFailed: Boolean;
  RestoreSucceeded: Boolean;
  RecoveryRestoredFiles: Boolean;
  UpgradeFailureDetails: String;
  InstallerExitCode: Integer;
  InstallerOutput: String;
  RecoveryVersion: String;
  RecoveryMarkerCount: Integer;
  RecoveryUnchanged: Boolean;

function GetFileAttributesW(FileName: String): LongWord;
  external 'GetFileAttributesW@kernel32.dll stdcall';

function ValidDirectory(Directory: String): Boolean;
var
  Current: String;
  Attributes: LongWord;
begin
  Result := False;
  if (Length(Directory) < 4) or (Length(Directory) > 160) then Exit;
  if (Directory[2] <> ':') or (Directory[3] <> '\') then Exit;
  if (Pos('%', Directory) > 0) or (Pos(';', Directory) > 0) or
     (Pos('!', Directory) > 0) or (Pos('"', Directory) > 0) or
     (Pos(#13, Directory) > 0) or (Pos(#10, Directory) > 0) then Exit;
  Current := Directory;
  while Length(Current) > 3 do begin
    Attributes := GetFileAttributesW(Current);
    if (Attributes <> $FFFFFFFF) and ((Attributes and $400) <> 0) then Exit;
    Current := ExtractFileDir(Current);
  end;
  Result := True;
end;

function ValidVersionArgument(Version: String): Boolean;
var
  Index: Integer;
begin
  Result := False;
  if (Length(Version) = 0) or (Length(Version) > 128) then Exit;
  for Index := 1 to Length(Version) do begin
    if Pos(Version[Index], '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-+') = 0 then Exit;
  end;
  Result := True;
end;

function SameDirectory(Left, Right: String): Boolean;
begin
  Result := CompareText(AddBackslash(ExpandFileName(Left)),
    AddBackslash(ExpandFileName(Right))) = 0;
end;

function InitializeSetup: Boolean;
var
  RecordedLocation: String;
begin
  Result := False;
  HasPreviousInstallation := RegKeyExists(HKCU64, UninstallKey);
  if HasPreviousInstallation then begin
    if not RegQueryStringValue(HKCU64, UninstallKey, 'Inno Setup: App Path', PreviousDirectory) or
       not RegQueryStringValue(HKCU64, UninstallKey, 'DisplayVersion', PreviousVersion) then begin
      SuppressibleMsgBox(CustomMessage('InvalidPreviousInstallation'), mbError, MB_OK, IDOK);
      Exit;
    end;
    if not ValidDirectory(PreviousDirectory) or not ValidVersionArgument(PreviousVersion) or
       not DirExists(PreviousDirectory) then begin
      SuppressibleMsgBox(CustomMessage('InvalidPreviousInstallation'), mbError, MB_OK, IDOK);
      Exit;
    end;
    if RegQueryStringValue(HKCU64, UninstallKey, 'InstallLocation', RecordedLocation) then begin
      if not SameDirectory(PreviousDirectory, RecordedLocation) then begin
        SuppressibleMsgBox(CustomMessage('InvalidPreviousInstallation'), mbError, MB_OK, IDOK);
        Exit;
      end;
    end;
    { 0.1.0 is the supported upgrade baseline; prerelease data is never overwritten. }
    { 0.1.0 是受支持的升级基线；绝不覆盖旧预发布版本的数据。 }
    if Pos('0.1.0-', PreviousVersion) = 1 then begin
      SuppressibleMsgBox(FmtMessage(CustomMessage('LegacyInstallation'), [PreviousVersion]),
        mbError, MB_OK, IDOK);
      Exit;
    end;
  end;
  Result := True;
end;

procedure InitializeWizard;
begin
  if HasPreviousInstallation then begin
    WizardForm.DirEdit.ReadOnly := True;
    WizardForm.DirBrowseButton.Enabled := False;
    WizardForm.SelectDirLabel.Caption := FmtMessage(CustomMessage('UpgradeDirectory'), [
      PreviousVersion, '{#PackageVersion}']);
    WizardForm.SelectDirBrowseLabel.Caption := CustomMessage('UpgradeDirectoryLocked');
  end;
end;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo,
  MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := MemoDirInfo + NewLine + NewLine + MemoTasksInfo;
  if HasPreviousInstallation then
    Result := FmtMessage(CustomMessage('UpgradeReady'), [PreviousVersion, '{#PackageVersion}']) +
      NewLine + NewLine + Result;
end;

procedure InstallerLog(const S: String; const Error, FirstLine: Boolean);
begin
  Log(S);
  if Trim(S) <> '' then InstallerOutput := Copy(S, 1, 1024);
  if not Error then begin
    if Pos('PYRUDDER_RESTORED_VERSION=', S) = 1 then begin
      RecoveryVersion := Copy(S, Length('PYRUDDER_RESTORED_VERSION=') + 1, Length(S));
      RecoveryMarkerCount := RecoveryMarkerCount + 1;
    end else if S = 'PYRUDDER_RECOVERY_UNCHANGED' then begin
      RecoveryUnchanged := True;
      RecoveryMarkerCount := RecoveryMarkerCount + 1;
    end;
  end;
end;

function RunInstallerPhase(Phase: String; InstalledProgram: Boolean): Boolean;
var
  ProgramPath, Arguments: String;
begin
  InstallerExitCode := -1;
  InstallerOutput := '';
  RecoveryVersion := '';
  RecoveryMarkerCount := 0;
  RecoveryUnchanged := False;
  if InstalledProgram then
    ProgramPath := ExpandConstant('{app}\bin\pyrudder.exe')
  else
    ProgramPath := ExpandConstant('{tmp}\pyrudder-installer-helper.exe');
  Arguments := '__installer --root "' + ExpandConstant('{app}') + '" --phase ' + Phase;
  if HasPreviousInstallation then
    Arguments := Arguments + ' --from-version "' + PreviousVersion + '"';
  try
    Result := ExecAndLogOutput(ProgramPath, Arguments, ExpandConstant('{tmp}'),
      SW_HIDE, ewWaitUntilTerminated, InstallerExitCode, @InstallerLog);
    if Result then Result := InstallerExitCode = 0;
  except
    InstallerOutput := GetExceptionMessage;
    Log(InstallerOutput);
    Result := False;
  end;
end;

function InstallerFailure(Phase: String): String;
begin
  Result := FmtMessage(CustomMessage('InstallerPhaseFailed'), [
    Phase, IntToStr(InstallerExitCode), InstallerOutput]);
end;

function RecoverPreviousInstallation: Boolean;
var
  RestoredVersion, RecordedDirectory, RecordedVersion: String;
begin
  Result := False;
  RecoveryRestoredFiles := False;
  if not RunInstallerPhase('rollback', False) then Exit;
  { Recovery output is a single strict marker, never an ad-hoc parse of the transaction JSON. }
  { 恢复输出使用单个严格标记，不临时拼凑解析事务 JSON。 }
  if (RecoveryMarkerCount <> 1) or ((RecoveryVersion = '') and not RecoveryUnchanged) then begin
    InstallerOutput := 'Invalid recovery result / 恢复结果无效';
    Exit;
  end;
  RestoredVersion := RecoveryVersion;
  if not RegQueryStringValue(HKCU64, UninstallKey, 'Inno Setup: App Path', RecordedDirectory) or
     not SameDirectory(RecordedDirectory, PreviousDirectory) then begin
    InstallerOutput := CustomMessage('InvalidPreviousInstallation');
    Exit;
  end;
  if RestoredVersion <> '' then begin
    if not ValidVersionArgument(RestoredVersion) then begin
      InstallerOutput := 'Invalid restored version / 恢复版本无效';
      Exit;
    end;
    { Keep the rolled-back journal until both files and the uninstall version agree. }
    { 文件和卸载登记版本一致前，保留已回滚日志以支持再次恢复。 }
    if not RegWriteStringValue(HKCU64, UninstallKey, 'DisplayVersion', RestoredVersion) then begin
      InstallerOutput := 'Cannot restore uninstall version / 无法恢复卸载登记版本';
      Exit;
    end;
  end;
  if not RegQueryStringValue(HKCU64, UninstallKey, 'DisplayVersion', RecordedVersion) or
     not ValidVersionArgument(RecordedVersion) then begin
    InstallerOutput := CustomMessage('InvalidPreviousInstallation');
    Exit;
  end;
  if (RestoredVersion <> '') and (RecordedVersion <> RestoredVersion) then begin
    InstallerOutput := 'Uninstall version changed during recovery / 恢复期间卸载登记版本发生变化';
    Exit;
  end;
  PreviousVersion := RecordedVersion;
  if RestoredVersion <> '' then begin
    if not RunInstallerPhase('recovery-complete', False) then Exit;
  end;
  RecoveryRestoredFiles := RestoredVersion <> '';
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Entry: TFindRec;
begin
  Result := '';
  if not ValidDirectory(ExpandConstant('{app}')) then begin
    Result := CustomMessage('InvalidDirectory');
    Exit;
  end;
  if HasPreviousInstallation then begin
    if not SameDirectory(ExpandConstant('{app}'), PreviousDirectory) then begin
      Result := FmtMessage(CustomMessage('UpgradeDirectoryChanged'), [PreviousDirectory]);
      Exit;
    end;
  end else begin
    if FindFirst(ExpandConstant('{app}\*'), Entry) then begin
      try
        repeat
          if (Entry.Name <> '.') and (Entry.Name <> '..') then begin
            Result := CustomMessage('EmptyDirectory');
            Exit;
          end;
        until not FindNext(Entry);
      finally
        FindClose(Entry);
      end;
    end;
  end;
  ExtractTemporaryFile('pyrudder-installer-helper.exe');
  { Recover a previous interrupted transaction before checking the current installation. }
  { 检查当前安装前，先恢复上次中断的更新事务。 }
  if HasPreviousInstallation and not UpgradePrepared and
     (DirExists(ExpandConstant('{app}\.pyrudder-upgrade-backup')) or
      FileExists(ExpandConstant('{app}\.pyrudder-upgrade-backup'))) then begin
    if not RecoverPreviousInstallation then begin
      Result := InstallerFailure('rollback');
      Exit;
    end;
  end;
  if not RunInstallerPhase('check', False) then begin
    Result := InstallerFailure('check');
    Exit;
  end;
  if HasPreviousInstallation and not UpgradePrepared then begin
    { A partially prepared transaction must also be offered rollback on failure. }
    { 准备阶段只完成一部分时，退出安装也必须尝试回滚。 }
    UpgradePrepared := True;
    if not RunInstallerPhase('prepare', False) then begin
      Result := InstallerFailure('prepare');
      Exit;
    end;
  end;
end;

function LocationArguments: String;
begin
  { Pin every path: inherited PYRUDDER_* variables must not redirect the installer. }
  { 固定所有路径：不允许继承的 PYRUDDER_* 环境变量重定向安装程序。 }
  Result := ExpandConstant('--home "{app}" --config-dir "{app}\config" ' +
    '--install-dir "{app}\bin" --shims-dir "{app}\shims" ' +
    '--runtimes-dir "{app}\runtimes" --downloads-dir "{app}\downloads" ' +
    '--cache-dir "{app}\cache" --temp-dir "{app}\temp" ');
end;

procedure InitializeInstallation;
begin
  { finish performs setup and republishes shims without resetting existing configuration. }
  { finish 执行安装初始化并重新发布 shim，不重置现有配置。 }
  if not RunInstallerPhase('finish', True) then
    RaiseException(InstallerFailure('finish'));
end;

function UpgradeFailureMessage: String;
begin
  if not RestoreSucceeded then
    Result := CustomMessage('UpgradeRollbackFailed') + #13#10#13#10 + UpgradeFailureDetails
  else if RecoveryRestoredFiles then
    Result := FmtMessage(CustomMessage('UpgradeFailedRecovered'), [PreviousVersion, UpgradeFailureDetails])
  else
    Result := FmtMessage(CustomMessage('UpgradeFailedPreserved'), [PreviousVersion, UpgradeFailureDetails]);
end;

procedure FailPostInstallUpdate(Details: String);
begin
  { Post-install event exceptions do not make Inno return failure; record the result explicitly. }
  { 后安装事件抛异常不会使 Inno 返回失败；必须显式记录结果和退出码。 }
  UpgradeFailed := True;
  UpgradeFailureDetails := Details;
  Log(Details);
  try
    RestoreSucceeded := RecoverPreviousInstallation;
  except
    RestoreSucceeded := False;
    Log(GetExceptionMessage);
  end;
  if RestoreSucceeded then UpgradePrepared := False;
  SuppressibleMsgBox(UpgradeFailureMessage, mbError, MB_OK, IDOK);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExitCode: Integer;
begin
  if UpgradeFailed then Exit;
  if (CurStep = ssPostInstall) and UpgradePrepared and not UpgradeCommitted then begin
    #ifdef InstallerTestFailBeforeCommit
      { Isolated test-only failure after Inno updated its uninstall metadata. }
      { 仅隔离测试使用：在 Inno 更新卸载登记后注入失败。 }
      FailPostInstallUpdate('Installer test failure before commit / 安装器测试：提交前失败');
      Exit;
    #endif
    if not RunInstallerPhase('commit', False) then begin
      FailPostInstallUpdate(InstallerFailure('commit'));
      Exit;
    end;
    UpgradeCommitted := True;
  end;
  { Persist PATH only after installer file publication/rollback has completed. }
  { 仅在安装器文件发布与回滚阶段结束后持久化 PATH。 }
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('systempath') then begin
    PathFailed := True;
    if ExecAndLogOutput(ExpandConstant('{app}\bin\pyrudder.exe'), LocationArguments + 'path add',
      ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode, nil) then
      PathFailed := ExitCode <> 0;
    if PathFailed then
      SuppressibleMsgBox(CustomMessage('PathAddFailed'), mbError, MB_OK, IDOK);
  end;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then begin
    if UpgradeFailed then begin
      WizardForm.FinishedHeadingLabel.Caption := CustomMessage('UpgradeFailedHeading');
      WizardForm.FinishedLabel.Caption := UpgradeFailureMessage;
    end else if PathFailed then
      WizardForm.FinishedLabel.Caption := CustomMessage('PathAddFailed')
    else if not WizardIsTaskSelected('systempath') then begin
      if HasPreviousInstallation then
        WizardForm.FinishedLabel.Caption := CustomMessage('UpgradeManualPathFinished')
      else
        WizardForm.FinishedLabel.Caption := FmtMessage(CustomMessage('ManualPathFinished'), [
           ExpandConstant('{app}\bin'), ExpandConstant('{app}\shims'),
           ExpandConstant('{app}\bin\pyrudder.exe')]);
    end;
  end;
end;

procedure DeinitializeSetup;
begin
  { Inno rolls back its files first; the helper then restores the old owned installation. }
  { Inno 先回滚本次文件；随后辅助程序恢复原有的受管理安装文件。 }
  if UpgradePrepared and not UpgradeCommitted then begin
    try
      RestoreSucceeded := RecoverPreviousInstallation;
    except
      RestoreSucceeded := False;
      Log(GetExceptionMessage);
    end;
    if RestoreSucceeded then UpgradePrepared := False
    else
      SuppressibleMsgBox(CustomMessage('UpgradeRollbackFailed'), mbError, MB_OK, IDOK);
  end;
end;

function GetCustomSetupExitCode: Integer;
begin
  if UpgradeFailed then Result := 11
  else if PathFailed then Result := 10
  else Result := 0;
end;

procedure RemoveOwnedPath;
var
  ExitCode: Integer;
begin
  { Restore the owned Store correction and remove machine PATH before deleting CLI files. }
  { 删除 CLI 文件前恢复拥有的商店修正并移除系统 PATH。 }
  if not ExecAndLogOutput(ExpandConstant('{app}\bin\pyrudder.exe'), LocationArguments + 'path remove',
    ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode, nil) then
    RaiseException(CustomMessage('PathRemoveFailed'));
  if ExitCode <> 0 then RaiseException(CustomMessage('PathRemoveFailed'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { This event runs after confirmation and before file removal; exceptions are fatal. }
  { 此事件在确认后、删除文件前执行；异常会中止卸载。 }
  if CurUninstallStep = usUninstall then RemoveOwnedPath;
  if CurUninstallStep = usPostUninstall then
    SuppressibleMsgBox(CustomMessage('DataPreserved'), mbInformation, MB_OK, IDOK);
end;
