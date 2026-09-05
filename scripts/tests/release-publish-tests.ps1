# Exercise the publication state machine with a local transport, never GitHub.
# 使用本地传输模拟发布状态机，绝不请求 GitHub。
#Requires -Version 7.0
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$releaseMockScriptDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $releaseMockScriptDirectory 'release-common.ps1')
$releaseMockVersion = [regex]::Match((Get-Content -LiteralPath (Join-Path $releaseMockScriptDirectory '../Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"\r\n]+)"').Groups[1].Value
$releaseMockCommit = '1234567890123456789012345678901234567890'
$releaseMockOtherCommit = 'abcdefabcdefabcdefabcdefabcdefabcdefabcd'
$releaseMockTag = "v$releaseMockVersion"
$releaseMockRoot = Join-Path $releaseMockScriptDirectory ('../target/release-publish-tests/' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $releaseMockRoot -Force)
$releaseMockUtf8 = [Text.UTF8Encoding]::new($false)
$releaseMockState = @{}

function New-ReleaseMockBundle {
    param([string]$Variant, [string]$SourceCommit)
    $releaseMockDirectory = Join-Path $releaseMockRoot ([Guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $releaseMockDirectory)
    $releaseMockNames = @(Get-ReleaseAssetNames -Version $releaseMockVersion)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $releaseMockArchive = [IO.Compression.ZipFile]::Open((Join-Path $releaseMockDirectory $releaseMockNames[0]), [IO.Compression.ZipArchiveMode]::Create)
    try {
        $releaseMockEntry = $releaseMockArchive.CreateEntry('BUILD-INFO.json')
        $releaseMockWriter = [IO.StreamWriter]::new($releaseMockEntry.Open(), $releaseMockUtf8)
        try {
            $releaseMockWriter.Write((@{ name = 'PyRudder'; version = $releaseMockVersion; source_commit = $SourceCommit; source_dirty = $false; fixture = $Variant } | ConvertTo-Json))
        } finally { $releaseMockWriter.Dispose() }
    } finally { $releaseMockArchive.Dispose() }
    [IO.File]::WriteAllText((Join-Path $releaseMockDirectory $releaseMockNames[2]), "setup fixture $Variant", $releaseMockUtf8)
    foreach ($releaseMockName in @($releaseMockNames[0], $releaseMockNames[2])) {
        $releaseMockHash = (Get-FileHash -LiteralPath (Join-Path $releaseMockDirectory $releaseMockName) -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText((Join-Path $releaseMockDirectory "$releaseMockName.sha256"), "$releaseMockHash  $releaseMockName`n", $releaseMockUtf8)
    }
    $releaseMockDirectory
}

function Invoke-ReleaseMockTransport {
    param([string[]]$Arguments, [switch]$AllowNotFound)
    if ($Arguments -contains '--clobber' -or $Arguments -contains '--force') { throw 'Unsafe overwrite option / 不安全的覆盖选项' }
    if ($Arguments[0] -eq 'api') {
        if ($Arguments[1] -eq '--method' -and $Arguments[2] -eq 'POST') {
            if ($releaseMockState.TagCommit) { throw 'Cannot replace a tag / 不能替换标签' }
            if ($Arguments[7] -cne "sha=$releaseMockCommit") { throw 'Incorrect tag target / 标签目标错误' }
            $releaseMockState.TagCommit = $releaseMockCommit
            $releaseMockState.Mutations++
            return '{}'
        }
        if ($Arguments[1] -match '/releases/tags/') {
            if ($releaseMockState.Release) { return ($releaseMockState.Release | ConvertTo-Json -Depth 8) }
        } elseif ($Arguments[1] -match '/git/ref/tags/') {
            if ($releaseMockState.TagCommit) { return (@{ object = @{ type = 'commit'; sha = $releaseMockState.TagCommit } } | ConvertTo-Json) }
        } else { throw 'Unexpected mock API request / 未预期的模拟 API 请求' }
        if ($AllowNotFound) { return $null }
        throw 'Unexpected missing remote object / 意外缺少远程对象'
    }
    if ($Arguments[0] -ne 'release') { throw 'Unexpected command / 未预期的命令' }
    switch ($Arguments[1]) {
        'create' {
            if ($releaseMockState.Release) { throw 'Release already exists / 发布已存在' }
            $releaseMockTargetIndex = [Array]::IndexOf($Arguments, '--target')
            if ($releaseMockTargetIndex -lt 0 -or $Arguments[$releaseMockTargetIndex + 1] -cne $releaseMockCommit -or $Arguments -notcontains '--verify-tag') {
                throw 'Explicit source target required / 必须显式指定源码提交'
            }
            $releaseMockState.Release = [pscustomobject]@{ tag_name = $releaseMockTag; target_commitish = $releaseMockCommit; draft = $true; prerelease = $true; assets = @() }
            $releaseMockState.Mutations++
        }
        'download' {
            $releaseMockName = $Arguments[[Array]::IndexOf($Arguments, '--pattern') + 1]
            $releaseMockDestination = $Arguments[[Array]::IndexOf($Arguments, '--dir') + 1]
            Copy-Item -LiteralPath $releaseMockState.Files[$releaseMockName] -Destination (Join-Path $releaseMockDestination $releaseMockName)
        }
        'upload' {
            $releaseMockUpload = $Arguments[3]
            $releaseMockName = [IO.Path]::GetFileName($releaseMockUpload)
            if ($releaseMockState.Files.ContainsKey($releaseMockName)) { throw 'Cannot replace an asset / 不能替换附件' }
            $releaseMockStored = Join-Path $releaseMockState.Directory $releaseMockName
            Copy-Item -LiteralPath $releaseMockUpload -Destination $releaseMockStored
            $releaseMockState.Files[$releaseMockName] = $releaseMockStored
            $releaseMockState.Release.assets += [pscustomobject]@{
                name = $releaseMockName; size = (Get-Item -LiteralPath $releaseMockStored).Length
                digest = 'sha256:' + (Get-FileHash -LiteralPath $releaseMockStored -Algorithm SHA256).Hash.ToLowerInvariant()
            }
            $releaseMockState.Mutations++
        }
        'edit' {
            if ($Arguments -notcontains '--draft=false' -or $Arguments -notcontains '--prerelease=true' -or $Arguments -notcontains '--latest=false') {
                throw 'Incorrect publication flags / 发布标记错误'
            }
            $releaseMockState.Release.draft = $false
            $releaseMockState.Mutations++
        }
        default { throw 'Unexpected release operation / 未预期的发布操作' }
    }
    ''
}

# Replace only the network function in memory; all production validation/control flow is retained.
# 仅在内存中替换网络函数，保留全部生产校验及控制流程。
$releaseMockPublishPath = Join-Path $releaseMockScriptDirectory 'release-publish.ps1'
$releaseMockText = Get-Content -LiteralPath $releaseMockPublishPath -Raw
$releaseMockTokens = $null
$releaseMockErrors = $null
$releaseMockAst = [Management.Automation.Language.Parser]::ParseInput($releaseMockText, [ref]$releaseMockTokens, [ref]$releaseMockErrors)
if ($releaseMockErrors) { throw 'Publication script parsing failed / 发布脚本语法解析失败' }
$releaseMockFunction = $releaseMockAst.Find({ param($releaseNode) $releaseNode -is [Management.Automation.Language.FunctionDefinitionAst] -and $releaseNode.Name -eq 'Invoke-ReleaseGh' }, $false)
if (-not $releaseMockFunction) { throw 'Missing network function / 缺少网络函数' }
$releaseMockReplacement = 'function Invoke-ReleaseGh { param([string[]]$Arguments, [switch]$AllowNotFound) Invoke-ReleaseMockTransport -Arguments $Arguments -AllowNotFound:$AllowNotFound }'
$releaseMockText = $releaseMockText.Remove($releaseMockFunction.Extent.StartOffset, $releaseMockFunction.Extent.EndOffset - $releaseMockFunction.Extent.StartOffset).Insert($releaseMockFunction.Extent.StartOffset, $releaseMockReplacement)
$releaseMockText = $releaseMockText.Replace('$PSScriptRoot', '$releaseMockScriptDirectory')
$releaseMockPublish = [scriptblock]::Create($releaseMockText)
$releaseMockFirstBundle = New-ReleaseMockBundle -Variant 'first' -SourceCommit $releaseMockCommit
$releaseMockSecondBundle = New-ReleaseMockBundle -Variant 'rebuilt' -SourceCommit $releaseMockCommit
$releaseMockOtherBundle = New-ReleaseMockBundle -Variant 'other-commit' -SourceCommit $releaseMockOtherCommit
$releaseMockRemoteDirectory = Join-Path $releaseMockRoot 'remote'
[void](New-Item -ItemType Directory -Path $releaseMockRemoteDirectory)
$releaseMockState = @{ TagCommit = $null; Release = $null; Mutations = 0; Files = @{}; Directory = $releaseMockRemoteDirectory }
$releaseMockSavedToken = $env:GH_TOKEN
$releaseMockSavedSummary = $env:GITHUB_STEP_SUMMARY
try {
    $env:GH_TOKEN = 'offline-test-placeholder'
    $env:GITHUB_STEP_SUMMARY = ''
    $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockFirstBundle
    if ($releaseMockState.Mutations -ne 7 -or $releaseMockState.Release.draft -or $releaseMockState.Files.Count -ne 4) { throw 'First publication failed / 首次发布失败' }
    $releaseMockOriginalAssets = @($releaseMockState.Release.assets)
    $releaseMockOriginalFiles = $releaseMockState.Files.Clone()
    $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockSecondBundle
    if ($releaseMockState.Mutations -ne 7) { throw 'Same-commit rerun changed published assets / 同提交重跑修改了公开附件' }
    $releaseMockRejected = $false
    try { $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockOtherCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockOtherBundle }
    catch { $releaseMockRejected = $true }
    if (-not $releaseMockRejected -or $releaseMockState.Mutations -ne 7) { throw 'Different commit was not safely rejected / 未安全拒绝不同提交' }
    $releaseMockNames = @(Get-ReleaseAssetNames -Version $releaseMockVersion)
    $releaseMockState.Release.draft = $true
    $releaseMockState.Release.assets = @($releaseMockOriginalAssets | Where-Object name -CEQ $releaseMockNames[0])
    $releaseMockState.Files = @{ ($releaseMockNames[0]) = $releaseMockOriginalFiles[$releaseMockNames[0]] }
    $releaseMockResumedDirectory = Join-Path $releaseMockRoot 'resumed'
    [void](New-Item -ItemType Directory -Path $releaseMockResumedDirectory)
    $releaseMockState.Directory = $releaseMockResumedDirectory
    $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockSecondBundle
    if ($releaseMockState.Mutations -ne 11 -or $releaseMockState.Release.draft -or $releaseMockState.Files.Count -ne 4) { throw 'Partial draft did not safely resume / 未安全恢复部分草稿' }
    $releaseMockState.Release.assets = @($releaseMockOriginalAssets | Where-Object name -CEQ $releaseMockNames[0])
    $releaseMockRejected = $false
    try { $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockSecondBundle }
    catch { $releaseMockRejected = $true }
    if (-not $releaseMockRejected -or $releaseMockState.Mutations -ne 11) { throw 'Incomplete published assets were modified / 已公开的不完整附件被修改' }
    $releaseMockState.Release.draft = $true
    $releaseMockState.Release.assets = @($releaseMockOriginalAssets | Where-Object name -CEQ $releaseMockNames[1])
    $releaseMockState.Files = @{ ($releaseMockNames[1]) = $releaseMockOriginalFiles[$releaseMockNames[1]] }
    $releaseMockRejected = $false
    try { $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockSecondBundle }
    catch { $releaseMockRejected = $true }
    if (-not $releaseMockRejected -or $releaseMockState.Mutations -ne 11) { throw 'Conflicting checksum was not safely rejected / 未安全拒绝冲突摘要' }
    Write-Output 'Publication simulation: 6 scenarios passed; no network access / 发布模拟：6 种场景通过，未访问网络'
} finally {
    $env:GH_TOKEN = $releaseMockSavedToken
    $env:GITHUB_STEP_SUMMARY = $releaseMockSavedSummary
}
