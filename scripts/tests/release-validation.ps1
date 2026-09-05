# Pure release-branch tests: no GitHub requests, tags or repository mutations.
# 纯版本分支测试：不请求 GitHub，不创建标签，不修改仓库。
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$releaseValidationScript = Join-Path $PSScriptRoot '../release-version.ps1'
$releaseValidationManifest = Join-Path $PSScriptRoot '../../Cargo.toml'
$releaseValidationText = Get-Content -LiteralPath $releaseValidationManifest -Raw
$releaseValidationVersion = [regex]::Match($releaseValidationText, '(?m)^version\s*=\s*"([^"\r\n]+)"').Groups[1].Value
if (-not $releaseValidationVersion) { throw 'Cannot read test version / 无法读取测试版本' }
# Mismatch fixtures must remain different when the workspace version changes.
# 工作区版本变化后，不匹配测试值仍须与其保持不同。
$releaseValidationOtherVersion = if ($releaseValidationVersion -ceq '0.0.0') { '0.0.1' } else { '0.0.0' }
foreach ($releaseValidationPrefix in @('release/')) {
    $releaseValidationResult = & $releaseValidationScript -Ref "refs/heads/$releaseValidationPrefix$releaseValidationVersion"
    if ($releaseValidationResult.Version -cne $releaseValidationVersion -or $releaseValidationResult.Tag -cne "v$releaseValidationVersion") {
        throw 'Accepted branch returned an incorrect version / 合法分支返回错误版本'
    }
}
foreach ($releaseValidationRef in @(
    'refs/heads/master', 'refs/heads/main', 'refs/heads/feature/example',
    "refs/tags/v$releaseValidationVersion", "refs/heads/release/v$releaseValidationVersion",
    "refs/heads/release/$releaseValidationOtherVersion", "refs/heads/release/$releaseValidationVersion/extra",
    "refs/heads/RELEASE/$releaseValidationVersion", "refs/heads/release/$releaseValidationVersion`n"
)) {
    $releaseValidationRejected = $false
    try { $null = & $releaseValidationScript -Ref $releaseValidationRef }
    catch { $releaseValidationRejected = $true }
    if (-not $releaseValidationRejected) { throw "Unexpectedly accepted ref / 错误接受分支: $releaseValidationRef" }
}
. (Join-Path $PSScriptRoot '../release-common.ps1')
$releaseTestDirectory = Join-Path $PSScriptRoot ("../../target/release-validation/" + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $releaseTestDirectory -Force)
$releaseTestUtf8 = [Text.UTF8Encoding]::new($false)
$releaseTestManifest = Join-Path $releaseTestDirectory 'Cargo.toml'
$releaseTestCount = 10

function Assert-ReleaseTestRejected {
    param([Parameter(Mandatory)][scriptblock]$Action)
    $releaseTestRejected = $false
    try { $null = & $Action } catch { $releaseTestRejected = $true }
    if (-not $releaseTestRejected) { throw 'Release safeguard unexpectedly accepted invalid input / 发布保护错误接受非法输入' }
}

foreach ($releaseTestVersion in @('0.1.0', '0.1.0-alpha.7', '1.2.3-beta.1', '1.2.3-rc.2', '1.2.3-0', '1.2.3+build.4')) {
    [IO.File]::WriteAllText($releaseTestManifest, "[workspace.package]`nversion = `"$releaseTestVersion`"`n", $releaseTestUtf8)
    $releaseTestResult = & $releaseValidationScript -Ref "refs/heads/release/$releaseTestVersion" -ManifestPath $releaseTestManifest
    if ($releaseTestResult.Version -cne $releaseTestVersion -or $releaseTestResult.Prerelease -ne $releaseTestVersion.Split('+')[0].Contains('-')) {
        throw 'Incorrect version/prerelease result / 版本或预发布判断错误'
    }
    $releaseTestCount++
}
foreach ($releaseTestVersion in @('01.2.3', '1.02.3', '1.2.03', '1.2.3-alpha.01', '1.2.3-', 'v1.2.3', '1.2', '1.2.3+', '1.2.3+build..4')) {
    [IO.File]::WriteAllText($releaseTestManifest, "[workspace.package]`nversion = `"$releaseTestVersion`"`n", $releaseTestUtf8)
    Assert-ReleaseTestRejected { & $releaseValidationScript -Ref "refs/heads/release/$releaseTestVersion" -ManifestPath $releaseTestManifest }
    $releaseTestCount++
}
$releaseTestNames = @(Get-ReleaseAssetNames -Version $releaseValidationVersion)
$releaseTestAssets = @($releaseTestNames | ForEach-Object { [pscustomobject]@{ name = $_ } })
Assert-ReleaseAssetSet -Assets $releaseTestAssets -ExpectedNames $releaseTestNames -Complete
Assert-ReleaseAssetSet -Assets @() -ExpectedNames $releaseTestNames
Assert-ReleaseTestRejected { Assert-ReleaseAssetSet -Assets @() -ExpectedNames $releaseTestNames -Complete }
Assert-ReleaseTestRejected { Assert-ReleaseAssetSet -Assets @($releaseTestAssets[0], $releaseTestAssets[0]) -ExpectedNames $releaseTestNames }
Assert-ReleaseTestRejected { Assert-ReleaseAssetSet -Assets @([pscustomobject]@{ name = 'unrelated.zip' }) -ExpectedNames $releaseTestNames }
$releaseTestCount += 5
$releaseTestCommit = '1234567890123456789012345678901234567890'
$releaseTestOtherCommit = 'abcdefabcdefabcdefabcdefabcdefabcdefabcd'
$releaseTestTag = "v$releaseValidationVersion"
$releaseTestRemote = [pscustomobject]@{
    tag_name = $releaseTestTag; target_commitish = $releaseTestCommit
    draft = $true; prerelease = $true; assets = @()
}
Assert-ReleaseIdentity -Release $releaseTestRemote -Tag $releaseTestTag -Commit $releaseTestCommit
Assert-ReleaseTestRejected { Assert-ReleaseIdentity -Release $releaseTestRemote -Tag $releaseTestTag -Commit $releaseTestOtherCommit }
Assert-ReleaseTestRejected { Assert-ReleaseIdentity -Release $releaseTestRemote -Tag "v$releaseValidationOtherVersion" -Commit $releaseTestCommit }
$releaseTestCount += 3
foreach ($releaseTestField in @('tag_name', 'target_commitish', 'draft', 'prerelease', 'assets')) {
    $releaseTestInvalid = $releaseTestRemote | ConvertTo-Json -Depth 8 | ConvertFrom-Json
    $releaseTestInvalid.PSObject.Properties.Remove($releaseTestField)
    Assert-ReleaseTestRejected { Assert-ReleaseIdentity -Release $releaseTestInvalid -Tag $releaseTestTag -Commit $releaseTestCommit }
    $releaseTestCount++
}
foreach ($releaseTestInvalidField in @(
    @{ Name = 'draft'; Value = 'false' }, @{ Name = 'draft'; Value = $null },
    @{ Name = 'prerelease'; Value = 1 }, @{ Name = 'prerelease'; Value = $null },
    @{ Name = 'assets'; Value = $null }, @{ Name = 'assets'; Value = [pscustomobject]@{ name = 'sample.zip' } }
)) {
    $releaseTestInvalid = $releaseTestRemote | ConvertTo-Json -Depth 8 | ConvertFrom-Json
    $releaseTestInvalid.($releaseTestInvalidField.Name) = $releaseTestInvalidField.Value
    Assert-ReleaseTestRejected { Assert-ReleaseIdentity -Release $releaseTestInvalid -Tag $releaseTestTag -Commit $releaseTestCommit }
    $releaseTestCount++
}
$releaseTestPayload = Join-Path $releaseTestDirectory 'sample.zip'
$releaseTestChecksum = "$releaseTestPayload.sha256"
[IO.File]::WriteAllText($releaseTestPayload, 'test payload', $releaseTestUtf8)
$releaseTestHash = (Get-FileHash -LiteralPath $releaseTestPayload -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText($releaseTestChecksum, "$releaseTestHash  sample.zip`n", $releaseTestUtf8)
Assert-ReleaseChecksum -PayloadPath $releaseTestPayload -ChecksumPath $releaseTestChecksum
[IO.File]::WriteAllText($releaseTestChecksum, "$releaseTestHash  wrong.zip`n", $releaseTestUtf8)
Assert-ReleaseTestRejected { Assert-ReleaseChecksum -PayloadPath $releaseTestPayload -ChecksumPath $releaseTestChecksum }
[IO.File]::WriteAllText($releaseTestChecksum, "$releaseTestHash  sample.zip`n", $releaseTestUtf8)
[IO.File]::WriteAllText($releaseTestPayload, 'changed payload', $releaseTestUtf8)
Assert-ReleaseTestRejected { Assert-ReleaseChecksum -PayloadPath $releaseTestPayload -ChecksumPath $releaseTestChecksum }
$releaseTestCount += 3

function New-ReleaseTestArchive {
    param([Parameter(Mandatory)][bool]$Dirty)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $releaseTestArchivePath = Join-Path $releaseTestDirectory ([Guid]::NewGuid().ToString('N') + '.zip')
    $releaseTestArchive = [IO.Compression.ZipFile]::Open($releaseTestArchivePath, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $releaseTestEntry = $releaseTestArchive.CreateEntry('BUILD-INFO.json')
        $releaseTestWriter = [IO.StreamWriter]::new($releaseTestEntry.Open(), $releaseTestUtf8)
        try {
            $releaseTestWriter.Write((@{ name = 'PyRudder'; version = $releaseValidationVersion; source_commit = $releaseTestCommit; source_dirty = $Dirty } | ConvertTo-Json))
        } finally { $releaseTestWriter.Dispose() }
    } finally { $releaseTestArchive.Dispose() }
    $releaseTestArchivePath
}
$releaseTestCleanArchive = New-ReleaseTestArchive -Dirty $false
$releaseTestDirtyArchive = New-ReleaseTestArchive -Dirty $true
Assert-ReleaseArchive -Path $releaseTestCleanArchive -Version $releaseValidationVersion -Commit $releaseTestCommit
Assert-ReleaseTestRejected { Assert-ReleaseArchive -Path $releaseTestCleanArchive -Version $releaseValidationOtherVersion -Commit $releaseTestCommit }
Assert-ReleaseTestRejected { Assert-ReleaseArchive -Path $releaseTestCleanArchive -Version $releaseValidationVersion -Commit $releaseTestOtherCommit }
Assert-ReleaseTestRejected { Assert-ReleaseArchive -Path $releaseTestDirtyArchive -Version $releaseValidationVersion -Commit $releaseTestCommit }
$releaseTestCount += 4
Write-Output "Release safeguards: $releaseTestCount checks passed / 发布保护：$releaseTestCount 项通过"
