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
$releaseMockScenarioCount = 0

function Reset-ReleaseMockState {
    param([switch]$Draft)
    $releaseMockDirectory = Join-Path $releaseMockRoot ([Guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $releaseMockDirectory)
    $script:releaseMockState = @{
        TagCommit = $null; Release = $null; Mutations = 0; Files = @{}; Directory = $releaseMockDirectory
        ListReads = 0; ByIdReads = 0; DraftTagMisses = 0; Operations = [Collections.Generic.List[string]]::new()
    }
    if ($Draft) {
        $releaseMockState.TagCommit = $releaseMockCommit
        $releaseMockState.Release = [pscustomobject]@{
            id = 101; tag_name = $releaseMockTag; target_commitish = $releaseMockCommit
            draft = $true; prerelease = $true; assets = @()
        }
    }
}

function Assert-ReleaseMockRejected {
    param([Parameter(Mandatory)][scriptblock]$Action, [Parameter(Mandatory)][string]$MessagePattern, [int]$AllowedMutations = 0)
    $releaseMockBefore = $releaseMockState.Mutations
    $releaseMockFailure = $null
    try { $null = & $Action } catch { $releaseMockFailure = $_ }
    if (-not $releaseMockFailure -or $releaseMockFailure.Exception.Message -notmatch $MessagePattern) {
        throw "Expected rejection was not observed / 未观察到预期拒绝: $MessagePattern; $releaseMockFailure"
    }
    if ($releaseMockState.Mutations -ne ($releaseMockBefore + $AllowedMutations) -or $releaseMockState.Operations.Contains('publish')) {
        throw 'Failed validation changed or published remote state / 校验失败后修改或公开了远程状态'
    }
    $script:releaseMockScenarioCount++
}

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
            $releaseMockState.Operations.Add('create-tag')
            return '{}'
        }
        if ($Arguments[1] -match '/releases/tags/') {
            # GitHub's tag endpoint does not expose drafts, even to their creator.
            # GitHub 的标签查询接口不会向草稿创建者返回草稿。
            if ($releaseMockState.Release -and $releaseMockState.Release.PSObject.Properties.Name -contains 'draft' -and $releaseMockState.Release.draft -ceq $false) {
                return ($releaseMockState.Release | ConvertTo-Json -Depth 8)
            }
            if ($releaseMockState.Release) { $releaseMockState.DraftTagMisses++ }
        } elseif ($Arguments[1] -match '/releases\?per_page=100$') {
            if ($Arguments -notcontains '--paginate' -or $Arguments -notcontains '--slurp' -or $AllowNotFound) {
                throw 'Release discovery must retain all pages and failures / 发布查询必须保留全部分页及错误'
            }
            $releaseMockState.ListReads++
            if ($releaseMockState.ContainsKey('ListError')) { throw $releaseMockState.ListError }
            if ($releaseMockState.ContainsKey('ListResponse')) { return $releaseMockState.ListResponse }
            if ($releaseMockState.ContainsKey('InvisibleAfterCreate') -and $releaseMockState.Release) { return '[[]]' }
            if ($releaseMockState.ContainsKey('InvisibleAtVerification') -and $releaseMockState.Files.Count -eq 4) { return '[[]]' }
            $releaseMockEntries = @(if ($releaseMockState.Release) { $releaseMockState.Release })
            if ($releaseMockState.ContainsKey('DuplicateDraft')) {
                $releaseMockEntries += [pscustomobject]@{ id = 102; tag_name = $releaseMockTag }
            }
            $releaseMockPageJson = ConvertTo-Json -InputObject @($releaseMockEntries) -Depth 8 -Compress
            if ($releaseMockState.ContainsKey('PageTwo')) {
                return ('[[{"id":99,"tag_name":"v0.0.1"}],' + $releaseMockPageJson + ']')
            }
            return ('[' + $releaseMockPageJson + ']')
        } elseif ($Arguments[1] -match '/releases/([1-9][0-9]*)$') {
            if ($AllowNotFound) { throw 'Numeric release lookup must not hide errors / 数字 ID 查询不得隐藏错误' }
            $releaseMockState.ByIdReads++
            if ($releaseMockState.ContainsKey('ByIdError')) { throw $releaseMockState.ByIdError }
            if ($releaseMockState.ContainsKey('ByIdResponse')) { return $releaseMockState.ByIdResponse }
            if ($releaseMockState.Release -and $Matches[1] -ceq [string]$releaseMockState.Release.id) {
                return ($releaseMockState.Release | ConvertTo-Json -Depth 8)
            }
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
            if ($Arguments -notcontains '--draft') { throw 'Creation must remain private until verified / 校验完成前发布必须保持草稿' }
            $releaseMockState.Release = [pscustomobject]@{ id = 101; tag_name = $releaseMockTag; target_commitish = $releaseMockCommit; draft = $true; prerelease = $true; assets = @() }
            $releaseMockState.Mutations++
            $releaseMockState.Operations.Add('create-draft')
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
            $releaseMockState.Operations.Add('upload')
        }
        'edit' {
            if ($Arguments -notcontains '--draft=false' -or $Arguments -notcontains '--prerelease=true' -or $Arguments -notcontains '--latest=false') {
                throw 'Incorrect publication flags / 发布标记错误'
            }
            $releaseMockState.Release.draft = $false
            $releaseMockState.Mutations++
            $releaseMockState.Operations.Add('publish')
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
Reset-ReleaseMockState
$releaseMockSavedToken = $env:GH_TOKEN
$releaseMockSavedSummary = $env:GITHUB_STEP_SUMMARY
try {
    $env:GH_TOKEN = 'offline-test-placeholder'
    $env:GITHUB_STEP_SUMMARY = ''
    $null = & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockFirstBundle
    if ($releaseMockState.Mutations -ne 7 -or $releaseMockState.Release.draft -or $releaseMockState.Files.Count -ne 4 -or
        $releaseMockState.DraftTagMisses -ne 2 -or $releaseMockState.ListReads -ne 3 -or $releaseMockState.ByIdReads -ne 2) {
        throw 'First publication did not discover its draft correctly / 首次发布未正确查询自己的草稿'
    }
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
    $releaseMockScenarioCount += 6

    $releaseMockRun = {
        & $releaseMockPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockFirstBundle
    }
    foreach ($releaseMockUseSecondPage in @($false, $true)) {
        Reset-ReleaseMockState -Draft
        if ($releaseMockUseSecondPage) { $releaseMockState.PageTwo = $true }
        $null = & $releaseMockRun
        if ($releaseMockState.Mutations -ne 5 -or $releaseMockState.Release.draft -or $releaseMockState.Files.Count -ne 4 -or
            $releaseMockState.ListReads -ne 2 -or $releaseMockState.ByIdReads -ne 2) {
            throw 'Existing draft or pagination recovery failed / 已有草稿或分页恢复失败'
        }
        $releaseMockScenarioCount++
    }

    Reset-ReleaseMockState -Draft
    $releaseMockState.DuplicateDraft = $true
    Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern 'Multiple releases use this tag'
    foreach ($releaseMockError in @('HTTP 403: forbidden', 'HTTP 404: inaccessible repository', 'HTTP 429: rate limited')) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.ListError = $releaseMockError
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern ([regex]::Escape($releaseMockError))
    }
    foreach ($releaseMockError in @('HTTP 404: draft disappeared', 'HTTP 403: draft permission denied')) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.ByIdError = $releaseMockError
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern ([regex]::Escape($releaseMockError))
    }
    foreach ($releaseMockInvalidList in @(
        @{ Json = ''; Pattern = 'Empty release list response' },
        @{ Json = 'null'; Pattern = 'Invalid paginated release list' },
        @{ Json = '{}'; Pattern = 'Invalid paginated release list' },
        @{ Json = '[{}]'; Pattern = 'Invalid release list page' },
        @{ Json = '[[null]]'; Pattern = 'Release list entry has no tag' },
        @{ Json = '[[{"id":101}]]'; Pattern = 'Release list entry has no tag' },
        @{ Json = ('[[{"id":"bad/id","tag_name":"' + $releaseMockTag + '"}]]'); Pattern = 'Invalid release ID' },
        @{ Json = ('[[{"id":0,"tag_name":"' + $releaseMockTag + '"}]]'); Pattern = 'Invalid release ID' }
    )) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.ListResponse = $releaseMockInvalidList.Json
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern $releaseMockInvalidList.Pattern
    }
    foreach ($releaseMockInvalidById in @(
        @{ Json = ''; Pattern = 'Empty draft release response' },
        @{ Json = 'null'; Pattern = 'Draft release identity changed' },
        @{ Json = ('{"id":102,"tag_name":"' + $releaseMockTag + '"}'); Pattern = 'Draft release identity changed' },
        @{ Json = '{"id":101,"tag_name":"v0.0.1"}'; Pattern = 'Draft release identity changed' }
    )) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.ByIdResponse = $releaseMockInvalidById.Json
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern $releaseMockInvalidById.Pattern
    }
    foreach ($releaseMockMissingField in @('target_commitish', 'draft', 'prerelease', 'assets')) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.Release.PSObject.Properties.Remove($releaseMockMissingField)
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern 'Release response is missing required state'
    }
    foreach ($releaseMockInvalidField in @(
        @{ Name = 'draft'; Value = 'false' }, @{ Name = 'draft'; Value = $null },
        @{ Name = 'prerelease'; Value = 'true' }, @{ Name = 'prerelease'; Value = 0 },
        @{ Name = 'assets'; Value = $null }, @{ Name = 'assets'; Value = [pscustomobject]@{ name = 'sample.zip' } }
    )) {
        Reset-ReleaseMockState -Draft
        $releaseMockState.Release.($releaseMockInvalidField.Name) = $releaseMockInvalidField.Value
        Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern 'Release response has invalid state types'
    }

    # Missing objects after creation or final verification must never become public.
    # 创建后或最终复核时缺少对象，不得继续公开发布。
    Reset-ReleaseMockState
    $releaseMockState.InvisibleAfterCreate = $true
    Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern 'Expected release is not visible' -AllowedMutations 2
    if ($releaseMockState.Files.Count -ne 0 -or -not $releaseMockState.Release.draft) { throw 'Invisible draft received assets / 不可见草稿接收了附件' }
    Reset-ReleaseMockState
    $releaseMockState.InvisibleAtVerification = $true
    Assert-ReleaseMockRejected -Action $releaseMockRun -MessagePattern 'Expected release is not visible' -AllowedMutations 6
    if ($releaseMockState.Files.Count -ne 4 -or -not $releaseMockState.Release.draft) { throw 'Unverified draft was not preserved / 未保留未验证草稿' }

    # Reintroduce the former lookup in memory to prove this fixture catches the reported bug.
    # 仅在内存中恢复旧查询逻辑，证明本测试能捕获本次报告的缺陷。
    $releaseMockLegacyAst = [Management.Automation.Language.Parser]::ParseInput($releaseMockText, [ref]$releaseMockTokens, [ref]$releaseMockErrors)
    $releaseMockLegacyFunction = $releaseMockLegacyAst.Find({ param($releaseNode) $releaseNode -is [Management.Automation.Language.FunctionDefinitionAst] -and $releaseNode.Name -eq 'Get-ReleaseRemote' }, $false)
    if (-not $releaseMockLegacyFunction) { throw 'Missing release lookup function / 缺少发布查询函数' }
    $releaseMockLegacyReplacement = @'
function Get-ReleaseRemote {
    param([switch]$Required)
    $releaseRemoteJson = Invoke-ReleaseGh -Arguments @('api', "repos/$Repository/releases/tags/$releaseTag") -AllowNotFound
    if ($releaseRemoteJson) { $releaseRemoteJson | ConvertFrom-Json }
}
'@
    $releaseMockLegacyText = $releaseMockText.Remove($releaseMockLegacyFunction.Extent.StartOffset, $releaseMockLegacyFunction.Extent.EndOffset - $releaseMockLegacyFunction.Extent.StartOffset).Insert($releaseMockLegacyFunction.Extent.StartOffset, $releaseMockLegacyReplacement)
    $releaseMockLegacyPublish = [scriptblock]::Create($releaseMockLegacyText)
    Reset-ReleaseMockState
    Assert-ReleaseMockRejected -Action {
        & $releaseMockLegacyPublish -Repository 'example/PyRudder' -Commit $releaseMockCommit -Ref "refs/heads/release/$releaseMockVersion" -DistDirectory $releaseMockFirstBundle
    } -MessagePattern "Cannot bind argument to parameter 'Release' because it is null" -AllowedMutations 2
    if ($releaseMockState.DraftTagMisses -ne 1 -or $releaseMockState.ListReads -ne 0 -or $releaseMockState.Files.Count -ne 0) {
        throw 'Legacy failure did not reproduce the draft lookup bug / 旧版失败未复现草稿查询缺陷'
    }
    Write-Output "Publication simulation: $releaseMockScenarioCount scenarios passed; legacy failure reproduced; no network access / 发布模拟：$releaseMockScenarioCount 种场景通过，已复现旧版缺陷，未访问网络"
} finally {
    $env:GH_TOKEN = $releaseMockSavedToken
    $env:GITHUB_STEP_SUMMARY = $releaseMockSavedSummary
}
