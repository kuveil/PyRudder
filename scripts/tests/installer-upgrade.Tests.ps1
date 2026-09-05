# Exercise two explicit installer payloads with a disposable uninstall identity and no PATH writes.
# 使用显式提供的两份安装内容和独立卸载标识验证更新，不写入 PATH。
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$OldPayload,
    [Parameter(Mandatory)][string]$NewPayload,
    [Parameter(Mandatory)][string]$InnoCompiler,
    [Parameter(Mandatory)][string]$PythonDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'This test requires Windows / 此测试仅支持 Windows' }
$upgradeTestRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$upgradeTestIdentity = [Guid]::NewGuid().ToString('N')
$upgradeTestAppId = "PyRudder.InstallerTest.$upgradeTestIdentity"
$upgradeTestRoot = Join-Path $upgradeTestRepository "target/installer-upgrade-tests/$upgradeTestIdentity"
$upgradeTestInstall = Join-Path $upgradeTestRoot 'installation'
$upgradeTestOutput = Join-Path $upgradeTestRoot 'packages'
$upgradeTestKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\${upgradeTestAppId}_is1"
$upgradeTestSource = Join-Path $upgradeTestRepository 'scripts/installer/pyrudder.iss'
$upgradeTestUtf8 = [Text.UTF8Encoding]::new($false)
$upgradeTestAssertions = 0

function Assert-InstallerTest {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
    $script:upgradeTestAssertions++
}

function Resolve-TestInput {
    param([string]$Path, [bool]$Directory)
    $resolved = (Resolve-Path -LiteralPath $Path).Path
    $item = Get-Item -LiteralPath $resolved
    if ($item.PSIsContainer -ne $Directory) { throw "Invalid test input / 无效测试输入: $resolved" }
    if ($resolved.Contains('"') -or $resolved.Contains("`r") -or $resolved.Contains("`n")) {
        throw 'Unsupported input path / 不支持的输入路径'
    }
    return $resolved
}

function Invoke-TestProcess {
    param([string]$Program, [string[]]$Arguments, [int]$TimeoutSeconds = 120, [switch]$ExpectFailure)
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Program
    $start.WorkingDirectory = $upgradeTestRoot
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    # Only this child loses inherited PyRudder overrides; the calling process is untouched.
    # 仅清除本次子进程继承的 PyRudder 覆盖变量，不改变调用进程环境。
    foreach ($name in @($start.Environment.Keys)) {
        if ($name.StartsWith('PYRUDDER_', [StringComparison]::OrdinalIgnoreCase)) {
            [void]$start.Environment.Remove($name)
        }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) { throw 'Cannot start test process / 无法启动测试进程' }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            # Do not kill an installer mid-transaction; preserve its PID and logs for inspection.
            # 不在事务中强制终止安装器；保留进程编号及日志供检查。
            throw "Test process timed out; inspect PID $($process.Id) / 测试进程超时，请检查进程及日志"
        }
        $result = [pscustomobject]@{
            ExitCode = $process.ExitCode
            Output = $stdout.GetAwaiter().GetResult()
            ErrorOutput = $stderr.GetAwaiter().GetResult()
        }
        if ($ExpectFailure -and $result.ExitCode -eq 0) {
            throw 'Installer unexpectedly accepted a rejected operation / 安装器错误接受了应拒绝的操作'
        }
        if (-not $ExpectFailure -and $result.ExitCode -ne 0) {
            throw "Test command failed ($($result.ExitCode)) / 测试命令失败: $($result.ErrorOutput) $($result.Output)"
        }
        return $result.Output.Trim()
    } finally { $process.Dispose() }
}

function Get-TestPathSnapshot {
    $values = [ordered]@{}
    foreach ($entry in @(
        @{ Name = 'User'; Hive = [Microsoft.Win32.RegistryHive]::CurrentUser; Key = 'Environment' },
        @{ Name = 'Machine'; Hive = [Microsoft.Win32.RegistryHive]::LocalMachine; Key = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment' }
    )) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey($entry.Hive, [Microsoft.Win32.RegistryView]::Registry64)
        $key = $null
        try {
            $key = $base.OpenSubKey($entry.Key, $false)
            if ($null -eq $key -or 'Path' -notin $key.GetValueNames()) {
                $values[$entry.Name] = $null
            } else {
                $values[$entry.Name] = @(
                    $key.GetValueKind('Path').ToString(),
                    $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
                )
            }
        } finally {
            if ($null -ne $key) { $key.Dispose() }
            $base.Dispose()
        }
    }
    return ($values | ConvertTo-Json -Depth 5 -Compress)
}

function Get-TestInstallation {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::CurrentUser, [Microsoft.Win32.RegistryView]::Registry64)
    $key = $null
    try {
        $key = $base.OpenSubKey($upgradeTestKey, $false)
        if ($null -eq $key) { return $null }
        return [pscustomobject]@{
            Directory = $key.GetValue('Inno Setup: App Path')
            Version = $key.GetValue('DisplayVersion')
            Tasks = $key.GetValue('Inno Setup: Selected Tasks')
            Uninstall = $key.GetValue('UninstallString')
        }
    } finally {
        if ($null -ne $key) { $key.Dispose() }
        $base.Dispose()
    }
}

function Test-SameDirectory {
    param([string]$Left, [string]$Right)
    return [IO.Path]::GetFullPath($Left).TrimEnd('\').Equals(
        [IO.Path]::GetFullPath($Right).TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)
}

function Invoke-InstalledTestCli {
    param([string[]]$Arguments)
    $output = Invoke-TestProcess -Program (Join-Path $upgradeTestInstall 'bin/pyrudder.exe') `
        -Arguments (@('--home', $upgradeTestInstall, '--json') + $Arguments)
    $result = $output | ConvertFrom-Json
    Assert-InstallerTest ($result.ok -eq $true) 'CLI test command failed / CLI 测试命令失败'
    return $result.data
}

function Assert-TestPythonRouting {
    $reported = Invoke-TestProcess -Program (Join-Path $upgradeTestInstall 'shims/python.exe') -Arguments @('--version')
    Assert-InstallerTest ($reported -ceq $upgradeTestPythonVersion) 'Python shim did not route to the registered Python / Python shim 未正确转发到已登记 Python'
    $pipShim = Join-Path $upgradeTestInstall 'shims/pip.exe'
    if (Test-Path -LiteralPath $pipShim -PathType Leaf) {
        $pipVersion = Invoke-TestProcess -Program $pipShim -Arguments @('--version')
        Assert-InstallerTest ($pipVersion -match '^pip\s+') 'pip shim did not execute / pip shim 无法运行'
    }
}

function Get-PayloadVersion {
    param([string]$Directory)
    $metadata = Get-Content -LiteralPath (Join-Path $Directory 'BUILD-INFO.json') -Raw | ConvertFrom-Json
    $version = [string]$metadata.version
    Assert-InstallerTest ($version -match '^\d+\.\d+\.\d+$' -and [version]$version -ge [version]'0.1.0') `
        'Test payloads must use stable versions >= 0.1.0 / 测试内容必须是至少 0.1.0 的正式版本'
    $reported = Invoke-TestProcess -Program (Join-Path $Directory 'bin/pyrudder.exe') -Arguments @('--version')
    Assert-InstallerTest ($reported -match ('(?i)^pyrudder\s+' + [regex]::Escape($version) + '$')) `
        'Payload metadata and binary versions differ / 安装内容元数据与程序版本不符'
    return $version
}

$OldPayload = Resolve-TestInput $OldPayload $true
$NewPayload = Resolve-TestInput $NewPayload $true
$InnoCompiler = Resolve-TestInput $InnoCompiler $false
$PythonDirectory = Resolve-TestInput $PythonDirectory $true
$upgradeTestPython = Join-Path $PythonDirectory 'python.exe'
Assert-InstallerTest (Test-Path -LiteralPath $upgradeTestPython -PathType Leaf) `
    'Provide an explicit Python directory containing python.exe / 请明确提供含 python.exe 的 Python 目录'
Assert-InstallerTest ($null -eq (Get-TestInstallation)) 'Disposable AppId already exists / 临时 AppId 已存在'
Assert-InstallerTest (-not (Test-Path -LiteralPath $upgradeTestRoot)) 'Test directory already exists / 测试目录已存在'
[void](New-Item -ItemType Directory -Path $upgradeTestOutput)
$upgradeTestOriginalPath = Get-TestPathSnapshot
$upgradeTestPythonHash = (Get-FileHash -LiteralPath $upgradeTestPython -Algorithm SHA256).Hash
$upgradeTestTimedOut = $false
$upgradeTestUninstalled = $false

try {
    $upgradeTestPythonVersion = Invoke-TestProcess -Program $upgradeTestPython -Arguments @('--version')
    $oldVersion = Get-PayloadVersion $OldPayload
    $newVersion = Get-PayloadVersion $NewPayload
    Assert-InstallerTest ([version]$newVersion -ge [version]$oldVersion) 'New test version must not be older / 新测试版本不能更旧'
    $oldHash = (Get-FileHash -LiteralPath (Join-Path $OldPayload 'bin/pyrudder.exe') -Algorithm SHA256).Hash
    $newHash = (Get-FileHash -LiteralPath (Join-Path $NewPayload 'bin/pyrudder.exe') -Algorithm SHA256).Hash
    Assert-InstallerTest ($oldHash -cne $newHash) 'Use distinct binaries to verify replacement / 请提供不同程序文件以验证替换'
    $sourceText = Get-Content -LiteralPath $upgradeTestSource -Raw
    Assert-InstallerTest ($sourceText.Contains('UsePreviousAppDir=yes') -and $sourceText.Contains('UsePreviousTasks=yes')) `
        'Previous directory/tasks must be reused / 必须沿用原目录和任务选择'
    Assert-InstallerTest ($sourceText.Contains('不会移除已有项') -and $sourceText.Contains('does not remove existing entries')) `
        'Unchecked upgrade PATH behavior must be documented bilingually / 必须双语说明取消勾选不移除旧 PATH'
    foreach ($payload in @(
        @{ Directory = $OldPayload; Version = $oldVersion; Name = 'old-setup'; Fail = $false },
        @{ Directory = $NewPayload; Version = $newVersion; Name = 'new-setup'; Fail = $false },
        @{ Directory = $NewPayload; Version = $newVersion; Name = 'failing-setup'; Fail = $true }
    )) {
        $compileArguments = @(
            '/Qp', "/DPayloadDir=$($payload.Directory)", "/DPackageVersion=$($payload.Version)",
            "/DAppIdentifier=$upgradeTestAppId", "/DDistDir=$upgradeTestOutput",
            "/DOutputName=$($payload.Name)"
        )
        if ($payload.Fail) { $compileArguments += '/DInstallerTestFailBeforeCommit=1' }
        $null = Invoke-TestProcess -Program $InnoCompiler -Arguments ($compileArguments + $upgradeTestSource)
    }
    # Both runs explicitly opt out of PATH; only the first run supplies a destination.
    # 两次均显式取消 PATH；只有第一次传入安装目录。
    $null = Invoke-TestProcess -Program (Join-Path $upgradeTestOutput 'old-setup.exe') -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=',
        "/DIR=$upgradeTestInstall", "/LOG=$(Join-Path $upgradeTestRoot 'install-old.log')"
    )
    $installed = Get-TestInstallation
    Assert-InstallerTest ($null -ne $installed -and (Test-SameDirectory $installed.Directory $upgradeTestInstall)) `
        'Old installer did not register the isolated directory / 旧安装包没有登记隔离目录'
    Assert-InstallerTest ($installed.Version -ceq $oldVersion -and [string]$installed.Tasks -eq '') `
        'Old installer did not record version/task choice / 旧安装包没有正确记录版本或任务选项'
    Assert-InstallerTest ((Get-TestPathSnapshot) -ceq $upgradeTestOriginalPath) 'Install changed PATH / 安装改变了 PATH'
    $null = Invoke-InstalledTestCli @('register', $PythonDirectory, '--alias', 'installer-test')
    $null = Invoke-InstalledTestCli @('global', 'installer-test')
    $null = Invoke-InstalledTestCli @('local', 'installer-test')
    $customRuntimes = Join-Path $upgradeTestRoot 'custom-runtimes'
    $customDownloads = Join-Path $upgradeTestRoot 'custom-downloads'
    $null = Invoke-InstalledTestCli @('config', 'set', 'paths.runtimes_dir', $customRuntimes)
    $null = Invoke-InstalledTestCli @('config', 'set', 'paths.downloads_dir', $customDownloads)
    [void](New-Item -ItemType Directory -Path $customRuntimes -Force)
    [void](New-Item -ItemType Directory -Path $customDownloads -Force)
    [IO.File]::WriteAllText((Join-Path $customRuntimes 'preserved-runtime.txt'), 'installer fixture', $upgradeTestUtf8)
    [IO.File]::WriteAllText((Join-Path $customDownloads 'preserved-download.txt'), 'installer fixture', $upgradeTestUtf8)
    $beforeList = (Invoke-InstalledTestCli @('list')) | ConvertTo-Json -Depth 30 -Compress
    $configPath = Join-Path $upgradeTestInstall 'config/config.toml'
    $globalPath = Join-Path $upgradeTestInstall 'config/global-version'
    $localPath = Join-Path $upgradeTestRoot '.python-version'
    $configHash = (Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash
    $globalHash = (Get-FileHash -LiteralPath $globalPath -Algorithm SHA256).Hash
    $localHash = (Get-FileHash -LiteralPath $localPath -Algorithm SHA256).Hash
    $runtimeSentinelHash = (Get-FileHash -LiteralPath (Join-Path $customRuntimes 'preserved-runtime.txt') -Algorithm SHA256).Hash
    $downloadSentinelHash = (Get-FileHash -LiteralPath (Join-Path $customDownloads 'preserved-download.txt') -Algorithm SHA256).Hash
    Assert-TestPythonRouting
    $null = Invoke-TestProcess -Program (Join-Path $upgradeTestOutput 'failing-setup.exe') -ExpectFailure -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=',
        "/LOG=$(Join-Path $upgradeTestRoot 'postinstall-failure.log')"
    )
    $restored = Get-TestInstallation
    Assert-InstallerTest ($null -ne $restored -and $restored.Version -ceq $oldVersion) `
        'Post-install failure did not restore the uninstall version / 后安装阶段失败后未恢复卸载登记版本'
    Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $upgradeTestInstall 'bin/pyrudder.exe') -Algorithm SHA256).Hash -ceq $oldHash) `
        'Post-install failure did not restore the old CLI / 后安装阶段失败后未恢复旧 CLI'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -ceq $configHash) `
        'Recovery changed user configuration / 恢复改变了用户配置'
    Assert-InstallerTest (((Invoke-InstalledTestCli @('list')) | ConvertTo-Json -Depth 30 -Compress) -ceq $beforeList) `
        'Recovery changed Python registrations / 恢复改变了 Python 登记信息'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $localPath -Algorithm SHA256).Hash -ceq $localHash) `
        'Recovery changed the local project pin / 恢复改变了项目固定版本'
    Assert-TestPythonRouting
    Assert-InstallerTest ((Get-TestPathSnapshot) -ceq $upgradeTestOriginalPath) 'Failed update changed PATH / 更新失败时改变了 PATH'
    $null = Invoke-TestProcess -Program (Join-Path $upgradeTestOutput 'new-setup.exe') -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=',
        "/LOG=$(Join-Path $upgradeTestRoot 'upgrade-new.log')"
    )
    $installed = Get-TestInstallation
    Assert-InstallerTest ($null -ne $installed -and (Test-SameDirectory $installed.Directory $upgradeTestInstall)) `
        'Upgrade failed to discover the original directory / 更新未自动识别原目录'
    Assert-InstallerTest ($installed.Version -ceq $newVersion -and [string]$installed.Tasks -eq '') `
        'Upgrade did not preserve the opted-out PATH choice / 更新未保留取消 PATH 的选择'
    Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $upgradeTestInstall 'bin/pyrudder.exe') -Algorithm SHA256).Hash -ceq $newHash) `
        'Upgrade did not replace the CLI binary / 更新没有替换 CLI 程序'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -ceq $configHash) `
        'Upgrade changed user configuration / 更新改变了用户配置'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $globalPath -Algorithm SHA256).Hash -ceq $globalHash) `
        'Upgrade changed the global selection / 更新改变了全局版本选择'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $localPath -Algorithm SHA256).Hash -ceq $localHash) `
        'Upgrade changed the local project pin / 更新改变了项目固定版本'
    $afterList = (Invoke-InstalledTestCli @('list')) | ConvertTo-Json -Depth 30 -Compress
    Assert-InstallerTest ($beforeList -ceq $afterList) 'Upgrade changed Python registrations / 更新改变了 Python 登记信息'
    Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $customDownloads 'preserved-download.txt') -Algorithm SHA256).Hash -ceq $downloadSentinelHash) `
        'Upgrade changed custom download data / 更新改变了自定义下载数据'
    Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $customRuntimes 'preserved-runtime.txt') -Algorithm SHA256).Hash -ceq $runtimeSentinelHash) `
        'Upgrade changed custom runtime data / 更新改变了自定义运行时数据'
    $templateHash = (Get-FileHash -LiteralPath (Join-Path $NewPayload 'bin/pyrudder-shim-console.exe') -Algorithm SHA256).Hash
    Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $upgradeTestInstall 'shims/python.exe') -Algorithm SHA256).Hash -ceq $templateHash) `
        'Upgrade did not republish the Python shim / 更新没有重新发布 Python shim'
    Assert-InstallerTest (-not (Test-Path -LiteralPath (Join-Path $upgradeTestInstall '.pyrudder-upgrade-backup'))) `
        'Successful upgrade left an unfinished transaction / 更新成功后仍有未完成事务'
    Assert-InstallerTest ((Get-TestPathSnapshot) -ceq $upgradeTestOriginalPath) 'Upgrade changed PATH / 更新改变了 PATH'
    Assert-TestPythonRouting
    # Repair also omits /TASKS, proving the stored opt-out is reused rather than reset.
    # 同版本修复同时省略 /TASKS，验证之前取消的选项被沿用而非重置。
    $null = Invoke-TestProcess -Program (Join-Path $upgradeTestOutput 'new-setup.exe') -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
        "/LOG=$(Join-Path $upgradeTestRoot 'repair-same-version.log')"
    )
    $repaired = Get-TestInstallation
    Assert-InstallerTest ($repaired.Version -ceq $newVersion -and [string]$repaired.Tasks -eq '') `
        'Repair did not reuse the stored PATH opt-out / 修复未沿用已保存的不配置 PATH 选择'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $configPath -Algorithm SHA256).Hash -ceq $configHash) `
        'Repair changed configuration / 修复改变了配置'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $globalPath -Algorithm SHA256).Hash -ceq $globalHash) `
        'Repair changed the global selection / 修复改变了全局版本选择'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $localPath -Algorithm SHA256).Hash -ceq $localHash) `
        'Repair changed the local project pin / 修复改变了项目固定版本'
    Assert-InstallerTest ((Get-TestPathSnapshot) -ceq $upgradeTestOriginalPath) 'Repair changed PATH / 修复改变了 PATH'
    Assert-TestPythonRouting
    if ([version]$newVersion -gt [version]$oldVersion) {
        $null = Invoke-TestProcess -Program (Join-Path $upgradeTestOutput 'old-setup.exe') -ExpectFailure -Arguments @(
            '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/TASKS=',
            "/LOG=$(Join-Path $upgradeTestRoot 'downgrade-rejected.log')"
        )
        Assert-InstallerTest ((Get-TestInstallation).Version -ceq $newVersion) 'Rejected downgrade changed version / 被拒绝的降级改变了版本'
        Assert-InstallerTest ((Get-FileHash -LiteralPath (Join-Path $upgradeTestInstall 'bin/pyrudder.exe') -Algorithm SHA256).Hash -ceq $newHash) `
            'Rejected downgrade changed the installed binary / 被拒绝的降级改变了安装程序文件'
    }
} catch {
    if ($_.Exception.Message.Contains('Test process timed out')) { $upgradeTestTimedOut = $true }
    throw
} finally {
    # Only invoke the generated test uninstaller; never delete registry keys or user Python manually.
    # 仅调用生成的测试卸载程序，绝不手动删除注册表项或用户 Python。
    $installed = Get-TestInstallation
    if ($null -ne $installed -and -not $upgradeTestTimedOut) {
        Assert-InstallerTest (Test-SameDirectory $installed.Directory $upgradeTestInstall) `
            'Refusing cleanup outside the test installation / 拒绝清理测试安装范围外的路径'
        $uninstaller = Join-Path $upgradeTestInstall 'unins000.exe'
        Assert-InstallerTest (Test-Path -LiteralPath $uninstaller -PathType Leaf) 'Test uninstaller is missing / 测试卸载程序缺失'
        Assert-InstallerTest ([string]$installed.Uninstall -like ('"' + $uninstaller + '"*')) `
            'Registered uninstaller is not the test uninstaller / 登记卸载程序并非测试卸载程序'
        $null = Invoke-TestProcess -Program $uninstaller -Arguments @(
            '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=$(Join-Path $upgradeTestRoot 'uninstall.log')"
        )
        $upgradeTestUninstalled = $true
        Assert-InstallerTest ($null -eq (Get-TestInstallation)) 'Test uninstall registration remains / 测试卸载登记仍然存在'
    }
    Assert-InstallerTest ((Get-TestPathSnapshot) -ceq $upgradeTestOriginalPath) 'Test changed PATH / 测试改变了 PATH'
    Assert-InstallerTest ((Get-FileHash -LiteralPath $upgradeTestPython -Algorithm SHA256).Hash -ceq $upgradeTestPythonHash) `
        'External Python changed during the test / 测试期间外部 Python 被改变'
    Write-Output "Installer test logs and retained fixture data / 安装测试日志及保留数据: $upgradeTestRoot"
}
Assert-InstallerTest $upgradeTestUninstalled 'Test installation was not uninstalled / 测试安装未卸载'
Write-Output "Installer upgrade tests passed: $upgradeTestAssertions checks / 安装器更新测试通过：$upgradeTestAssertions 项"
