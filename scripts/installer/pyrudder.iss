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

[Setup]
AppId=PyRudder.Alpha
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
UsePreviousAppDir=no
UsePreviousTasks=no
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
SetupMutex=PyRudderAlphaSetup

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Default.isl,ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
; Selected by default, but users and /TASKS may opt out of machine PATH changes.
; 默认勾选，用户及 /TASKS 参数均可取消系统 PATH 配置。
Name: "systempath"; Description: "{cm:SystemPathTask}"; GroupDescription: "{cm:SystemPathGroup}"

[Files]
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
chinesesimplified.EmptyDirectory=请选择一个新的空目录。此 alpha 不覆盖旧安装，也不自动迁移已有数据。
english.EmptyDirectory=Choose a new empty directory. This alpha does not overwrite existing installations or migrate data.
chinesesimplified.ExistingInstaller=已存在安装包管理的 PyRudder，请先在 Windows“已安装的应用”中卸载，再选择新的空目录安装。保留的数据不会自动迁移。
english.ExistingInstaller=An installer-managed PyRudder already exists. Uninstall it from Windows Installed apps first, then select a new empty directory. Retained data is not migrated automatically.
chinesesimplified.InvalidDirectory=请选择本地盘符下的普通目录（最多 160 字符），不要选择盘符根目录、链接目录，或含 %% ; ! 的目录。
english.InvalidDirectory=Choose an ordinary local drive directory (up to 160 characters), not a drive root, link, or path containing %% ; !.
chinesesimplified.SetupFailed=初始化失败，退出码 %1。请查看安装日志。请勿删除已有的 Python 或配置数据。
english.SetupFailed=Initialization failed with exit code %1. Check the setup log. Do not delete existing Python or configuration data.
chinesesimplified.PathRemoveFailed=无法恢复本安装清理的 WindowsApps 系统 PATH 项，或无法移除本安装添加的系统 PATH 项；CLI 文件尚未删除，卸载已中止。请重新运行卸载并完成管理员授权，详情见卸载日志。
english.PathRemoveFailed=Cannot restore the WindowsApps machine PATH entry cleaned by this installation or remove its owned machine PATH entries. CLI files were kept and uninstall stopped. Rerun uninstall and approve Administrator permission; see the uninstall log.
chinesesimplified.PathAddFailed=CLI 文件已安装，但系统 PATH 配置或 WindowsApps 冲突项清理失败。安装未完整成功；请查看安装日志，卸载后重新安装并完成管理员授权。
english.PathAddFailed=CLI files were installed, but machine PATH configuration or WindowsApps conflict cleanup failed. Installation is incomplete. Check the setup log, uninstall, then reinstall and approve Administrator permission.
chinesesimplified.DataPreserved=CLI 已卸载，本安装记录的 PATH 修改（如有）已撤销。配置、登记信息、shim 和托管 Python 数据保留在原目录；外部 Python 与商店应用文件未删除。
english.DataPreserved=The CLI was removed and any PATH changes recorded by this installation were undone. Configuration, registrations, shims and managed Python data remain in place; external Python and Store app files were not deleted.

[Messages]
chinesesimplified.WelcomeLabel2=本向导安装 PyRudder 命令行工具并允许选择安装目录。%n%n“加入系统 PATH”默认勾选，也可取消后自行配置。勾选后，安装器请求管理员授权，将 bin 和 shims 追加到系统 PATH（对所有用户生效），并仅移除系统 PATH 中当前账户的 WindowsApps 项，以避免 python 打开 Microsoft Store。不会写入用户 PATH，也不删除商店应用。%n%n安装后无桌面应用、托盘或服务，不自动安装 Python。这是未签名 alpha 测试版。
english.WelcomeLabel2=This wizard installs the PyRudder CLI and lets you choose its destination.%n%nAdd to system PATH is selected by default; clear it to configure PATH yourself. When selected, setup requests Administrator permission, appends bin and shims to system PATH for all users, and removes only the current account's WindowsApps entry from system PATH so python does not open Microsoft Store. User PATH and Store apps are unchanged.%n%nNo desktop app, tray or service; Python is not installed automatically. This is an unsigned alpha test build.
chinesesimplified.FinishedLabelNoIcons=PyRudder CLI 已安装并配置系统 PATH。请完全关闭并重开终端，直接使用 pyrudder 命令。%n%n登记已有 Python：%npyrudder register "D:\Python313" --alias work%npyrudder global work%n%n项目固定：pyrudder local work%n当前终端：pyrudder shell work%n%n安装器已自动处理当前账户的 WindowsApps 系统 PATH 冲突；不需要初始化脚本。
english.FinishedLabelNoIcons=PyRudder CLI is installed and machine PATH is configured. Fully close and reopen your terminal, then use pyrudder directly.%n%nRegister existing Python:%npyrudder register "D:\Python313" --alias work%npyrudder global work%n%nPin a project: pyrudder local work%nThis terminal: pyrudder shell work%n%nSetup automatically handled the current account's WindowsApps machine PATH conflict. No initialization script is needed.
chinesesimplified.ConfirmUninstall=卸载 PyRudder CLI？如果本安装曾修改系统 PATH，将请求管理员授权移除其添加的项，并恢复其清理的 WindowsApps 项；取消或失败时不删除 CLI。%n%n保留配置、登记信息、shim 和托管 Python；不删除外部 Python 或商店应用文件。
english.ConfirmUninstall=Remove the PyRudder CLI? If this installation changed system PATH, Administrator permission will be requested to remove its added entries and restore its cleaned WindowsApps entry. Cancellation or failure keeps the CLI files.%n%nConfiguration, registrations, shims and managed Python are preserved. External Python and Store app files are not deleted.

[Code]
var
  PathFailed: Boolean;

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
     (Pos('!', Directory) > 0) then Exit;
  Current := Directory;
  while Length(Current) > 3 do begin
    Attributes := GetFileAttributesW(Current);
    if (Attributes <> $FFFFFFFF) and ((Attributes and $400) <> 0) then Exit;
    Current := ExtractFileDir(Current);
  end;
  Result := True;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Entry: TFindRec;
begin
  Result := '';
  if RegKeyExists(HKCU64, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\PyRudder.Alpha_is1') then begin
    Result := CustomMessage('ExistingInstaller');
    Exit;
  end;
  if not ValidDirectory(ExpandConstant('{app}')) then begin
    Result := CustomMessage('InvalidDirectory');
    Exit;
  end;
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
var
  Arguments: String;
  ExitCode: Integer;
begin
  Arguments := LocationArguments + 'setup';
  if not ExecAndLogOutput(ExpandConstant('{app}\bin\pyrudder.exe'), Arguments,
    ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode, nil) then
    RaiseException(FmtMessage(CustomMessage('SetupFailed'), [IntToStr(ExitCode)]));
  if ExitCode <> 0 then
    RaiseException(FmtMessage(CustomMessage('SetupFailed'), [IntToStr(ExitCode)]));
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExitCode: Integer;
begin
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
    if PathFailed then
      WizardForm.FinishedLabel.Caption := CustomMessage('PathAddFailed')
    else if not WizardIsTaskSelected('systempath') then
      WizardForm.FinishedLabel.Caption := FmtMessage(CustomMessage('ManualPathFinished'), [
         ExpandConstant('{app}\bin'), ExpandConstant('{app}\shims'),
         ExpandConstant('{app}\bin\pyrudder.exe')]);
  end;
end;

function GetCustomSetupExitCode: Integer;
begin
  if PathFailed then Result := 10
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
