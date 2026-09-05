# Publish an explicitly validated source revision without moving tags or replacing assets.
# 发布经过明确校验的源码提交，不移动标签，不替换已有附件。
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('\A[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\z')][string]$Repository,
    [Parameter(Mandatory)][ValidatePattern('\A[0-9a-f]{40}\z')][string]$Commit,
    [Parameter(Mandatory)][string]$Ref,
    [string]$DistDirectory = (Join-Path $PSScriptRoot '../target/dist')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'release-common.ps1')
$releaseVersionInfo = & (Join-Path $PSScriptRoot 'release-version.ps1') -Ref $Ref
$releaseVersion = $releaseVersionInfo.Version
$releaseTag = $releaseVersionInfo.Tag
$releaseExpectedNames = @(Get-ReleaseAssetNames -Version $releaseVersion)
$releaseLocalAssets = @(Get-ChildItem -LiteralPath $DistDirectory -File | ForEach-Object { [pscustomobject]@{ name = $_.Name } })
Assert-ReleaseAssetSet -Assets $releaseLocalAssets -ExpectedNames $releaseExpectedNames -Complete
foreach ($releasePayloadName in @($releaseExpectedNames[0], $releaseExpectedNames[2])) {
    Assert-ReleaseChecksum -PayloadPath (Join-Path $DistDirectory $releasePayloadName) -ChecksumPath (Join-Path $DistDirectory "$releasePayloadName.sha256")
}
Assert-ReleaseArchive -Path (Join-Path $DistDirectory $releaseExpectedNames[0]) -Version $releaseVersion -Commit $Commit
if (-not $env:GH_TOKEN) { throw 'GH_TOKEN is required / 需要 GH_TOKEN' }

function Invoke-ReleaseGh {
    param([Parameter(Mandatory)][string[]]$Arguments, [switch]$AllowNotFound)
    $releaseProcessInfo = [Diagnostics.ProcessStartInfo]::new('gh')
    $releaseProcessInfo.UseShellExecute = $false
    $releaseProcessInfo.CreateNoWindow = $true
    $releaseProcessInfo.RedirectStandardOutput = $true
    $releaseProcessInfo.RedirectStandardError = $true
    foreach ($releaseArgument in $Arguments) { [void]$releaseProcessInfo.ArgumentList.Add($releaseArgument) }
    $releaseProcess = [Diagnostics.Process]::new()
    $releaseProcess.StartInfo = $releaseProcessInfo
    try {
        if (-not $releaseProcess.Start()) { throw 'Cannot start GitHub CLI / 无法启动 GitHub CLI' }
        $releaseOutputTask = $releaseProcess.StandardOutput.ReadToEndAsync()
        $releaseErrorTask = $releaseProcess.StandardError.ReadToEndAsync()
        $releaseProcess.WaitForExit()
        $releaseOutputText = $releaseOutputTask.GetAwaiter().GetResult()
        $releaseErrorText = $releaseErrorTask.GetAwaiter().GetResult()
        if ($releaseProcess.ExitCode -ne 0) {
            if ($AllowNotFound -and $releaseErrorText -match '\(HTTP 404\)') { return $null }
            throw "GitHub CLI failed / GitHub CLI 失败: $releaseErrorText"
        }
        $releaseOutputText
    } finally { $releaseProcess.Dispose() }
}

function Get-ReleaseRemote {
    $releaseRemoteJson = Invoke-ReleaseGh -Arguments @('api', "repos/$Repository/releases/tags/$releaseTag") -AllowNotFound
    if ($releaseRemoteJson) { $releaseRemoteJson | ConvertFrom-Json }
}

function Get-ReleaseTagCommit {
    $releaseRefJson = Invoke-ReleaseGh -Arguments @('api', "repos/$Repository/git/ref/tags/$releaseTag") -AllowNotFound
    if (-not $releaseRefJson) { return $null }
    $releaseObject = ($releaseRefJson | ConvertFrom-Json).object
    for ($releaseDepth = 0; $releaseDepth -lt 8; $releaseDepth++) {
        if ($releaseObject.type -ceq 'commit') { return $releaseObject.sha }
        if ($releaseObject.type -cne 'tag') { break }
        $releaseObject = ((Invoke-ReleaseGh -Arguments @('api', "repos/$Repository/git/tags/$($releaseObject.sha)")) | ConvertFrom-Json).object
    }
    throw 'Tag does not resolve to a commit / 标签不能解析为提交'
}

function Assert-ReleaseRemoteTag {
    $releaseRemoteCommit = Get-ReleaseTagCommit
    if ($releaseRemoteCommit -cne $Commit) {
        throw 'Existing tag points elsewhere; increment the version instead of moving the tag / 已有标签指向不同提交，请升版，不得移动标签'
    }
}

function Save-ReleaseAsset {
    param([Parameter(Mandatory)][object]$Asset, [Parameter(Mandatory)][string]$Directory)
    $null = Invoke-ReleaseGh -Arguments @('release', 'download', $releaseTag, '--repo', $Repository, '--dir', $Directory, '--pattern', $Asset.name)
    $releaseSavedPath = Join-Path $Directory $Asset.name
    if ((Get-Item -LiteralPath $releaseSavedPath).Length -ne $Asset.size) {
        throw "Downloaded asset size mismatch / 下载附件大小不匹配: $($Asset.name)"
    }
    # Verify GitHub's recorded digest when present, in addition to the published sidecar.
    # 除发布的摘要文件外，也校验 GitHub 提供的附件摘要（如存在）。
    if ($Asset.PSObject.Properties.Name -contains 'digest' -and $Asset.digest) {
        $releaseSavedDigest = 'sha256:' + (Get-FileHash -LiteralPath $releaseSavedPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($Asset.digest -cne $releaseSavedDigest) { throw 'GitHub asset digest mismatch / GitHub 附件摘要不匹配' }
    }
    $releaseSavedPath
}

$releaseExisting = Get-ReleaseRemote
$releaseExistingCommit = Get-ReleaseTagCommit
if ($releaseExisting) { Assert-ReleaseIdentity -Release $releaseExisting -Tag $releaseTag -Commit $Commit }
if ($releaseExistingCommit -and $releaseExistingCommit -cne $Commit) {
    throw 'This version tag already exists at another commit; increment the version / 版本标签已存在于其他提交，请提升版本号'
}
if ($releaseExisting -and -not $releaseExistingCommit) { throw 'Release exists without its expected tag / 发布缺少对应标签' }
$releaseWorkRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../target/release-publish'))
$releaseWork = Join-Path $releaseWorkRoot ([Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $releaseWork -Force)
$releaseUtf8 = [Text.UTF8Encoding]::new($false)

if (-not $releaseExistingCommit) {
    # An explicit SHA also works when the repository has no master/main branch.
    # 显式提交 SHA 同样适用于尚未建立 master/main 分支的仓库。
    $null = Invoke-ReleaseGh -Arguments @('api', '--method', 'POST', "repos/$Repository/git/refs", '-f', "ref=refs/tags/$releaseTag", '-f', "sha=$Commit")
}
Assert-ReleaseRemoteTag
if (-not $releaseExisting) {
    $releaseNotes = @"
## 中文

PyRudder $releaseVersion：适用于 Windows x64 的 Python 版本管理 CLI。

- 安装向导支持选择目录，并可选配置系统 PATH；安装后仅使用命令行。
- 使用 ``pyrudder register`` 接管已有 Python，使用 ``pyrudder available`` 上下选择并安装官方版本。
- 安装时可以指定目录，留空使用 PyRudder 下的默认目录；下载显示进度。
- 使用 ``pyrudder global`` 切换全局版本，使用 ``pyrudder local`` 固定项目版本。

推荐下载安装程序 ``pyrudder-$releaseVersion-windows-x64-Setup.exe``。也提供便携 ZIP 及 SHA256 摘要文件。此预览版本未进行代码签名，Windows 可能显示安全提醒。完整教程见中文 README。

## English

PyRudder $releaseVersion is a Python version manager for Windows x64, used entirely from the command line after setup.

- Choose the installation directory and optional system PATH integration in the setup wizard.
- Register existing Python installations with ``pyrudder register``, or choose official versions interactively with ``pyrudder available``.
- Choose a runtime directory or accept the default inside PyRudder; follow the download progress.
- Switch the global Python version with ``pyrudder global`` or pin a project with ``pyrudder local``.

The Setup executable is recommended. A portable ZIP and SHA256 checksum files are also available. This preview is not code-signed, so Windows may show a security warning. See the English README for the complete guide.
"@
    $releaseNotesPath = Join-Path $releaseWork 'release-notes.md'
    [IO.File]::WriteAllText($releaseNotesPath, $releaseNotes, $releaseUtf8)
    $releaseCreateArguments = @('release', 'create', $releaseTag, '--repo', $Repository, '--verify-tag', '--target', $Commit,
        '--draft', '--title', "PyRudder $releaseVersion", '--notes-file', $releaseNotesPath)
    if ($releaseVersionInfo.Prerelease) { $releaseCreateArguments += '--prerelease' }
    $null = Invoke-ReleaseGh -Arguments $releaseCreateArguments
    $releaseExisting = Get-ReleaseRemote
}
Assert-ReleaseIdentity -Release $releaseExisting -Tag $releaseTag -Commit $Commit
Assert-ReleaseAssetSet -Assets @($releaseExisting.assets) -ExpectedNames $releaseExpectedNames -Complete:(-not $releaseExisting.draft)
$releaseExistingDirectory = Join-Path $releaseWork 'existing'
[void](New-Item -ItemType Directory -Path $releaseExistingDirectory)
$releaseExistingPaths = @{}
foreach ($releaseAsset in $releaseExisting.assets) {
    $releaseExistingPaths[$releaseAsset.name] = Save-ReleaseAsset -Asset $releaseAsset -Directory $releaseExistingDirectory
}

foreach ($releasePayloadName in @($releaseExpectedNames[0], $releaseExpectedNames[2])) {
    $releaseChecksumName = "$releasePayloadName.sha256"
    $releaseChosenPayload = if ($releaseExistingPaths.ContainsKey($releasePayloadName)) { $releaseExistingPaths[$releasePayloadName] } else { Join-Path $DistDirectory $releasePayloadName }
    $releaseChosenChecksum = if ($releaseExistingPaths.ContainsKey($releaseChecksumName)) { $releaseExistingPaths[$releaseChecksumName] } else {
        # Resume a partial draft by hashing its existing payload, never replacing it.
        # 通过对草稿中已有文件计算摘要继续未完成发布，绝不替换原文件。
        $releaseGeneratedChecksum = Join-Path $releaseWork $releaseChecksumName
        $releaseGeneratedHash = (Get-FileHash -LiteralPath $releaseChosenPayload -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText($releaseGeneratedChecksum, "$releaseGeneratedHash  $releasePayloadName`n", $releaseUtf8)
        $releaseGeneratedChecksum
    }
    Assert-ReleaseChecksum -PayloadPath $releaseChosenPayload -ChecksumPath $releaseChosenChecksum
    if ($releasePayloadName.EndsWith('.zip', [StringComparison]::Ordinal)) {
        Assert-ReleaseArchive -Path $releaseChosenPayload -Version $releaseVersion -Commit $Commit
    }
    foreach ($releaseChosenPath in @($releaseChosenPayload, $releaseChosenChecksum)) {
        if (-not $releaseExistingPaths.ContainsKey([IO.Path]::GetFileName($releaseChosenPath))) {
            if (-not $releaseExisting.draft) { throw 'Refusing to modify a published release / 拒绝修改已公开发布' }
            $null = Invoke-ReleaseGh -Arguments @('release', 'upload', $releaseTag, $releaseChosenPath, '--repo', $Repository)
        }
    }
}

# Validate the complete remote set before changing a draft to a public release.
# 将草稿转为公开发布前，复核完整的远程附件集合。
Assert-ReleaseRemoteTag
$releaseComplete = Get-ReleaseRemote
Assert-ReleaseIdentity -Release $releaseComplete -Tag $releaseTag -Commit $Commit
Assert-ReleaseAssetSet -Assets @($releaseComplete.assets) -ExpectedNames $releaseExpectedNames -Complete
$releaseVerifiedDirectory = Join-Path $releaseWork 'verified'
[void](New-Item -ItemType Directory -Path $releaseVerifiedDirectory)
foreach ($releaseAsset in $releaseComplete.assets) { $null = Save-ReleaseAsset -Asset $releaseAsset -Directory $releaseVerifiedDirectory }
foreach ($releasePayloadName in @($releaseExpectedNames[0], $releaseExpectedNames[2])) {
    Assert-ReleaseChecksum -PayloadPath (Join-Path $releaseVerifiedDirectory $releasePayloadName) -ChecksumPath (Join-Path $releaseVerifiedDirectory "$releasePayloadName.sha256")
}
Assert-ReleaseArchive -Path (Join-Path $releaseVerifiedDirectory $releaseExpectedNames[0]) -Version $releaseVersion -Commit $Commit
if ($releaseComplete.draft) {
    $releasePrereleaseFlag = '--prerelease=' + $releaseVersionInfo.Prerelease.ToString().ToLowerInvariant()
    $releaseLatestFlag = '--latest=' + (-not $releaseVersionInfo.Prerelease).ToString().ToLowerInvariant()
    $null = Invoke-ReleaseGh -Arguments @('release', 'edit', $releaseTag, '--repo', $Repository, '--draft=false', $releasePrereleaseFlag, $releaseLatestFlag)
} elseif ($releaseComplete.prerelease -ne $releaseVersionInfo.Prerelease) {
    throw 'Existing published prerelease status differs; refusing to edit it / 已公开版本的预发布标记不符，拒绝修改'
}
$releaseUrl = "https://github.com/$Repository/releases/tag/$releaseTag"
Write-Output "Release verified / 发布已校验: $releaseUrl"
if ($env:GITHUB_STEP_SUMMARY) { Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value "[PyRudder $releaseVersion]($releaseUrl)" -Encoding utf8 }
