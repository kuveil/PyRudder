# Validate the branch and workspace version without contacting GitHub.
# 校验版本分支与工作区版本，不连接 GitHub。
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Ref,
    [string]$ManifestPath = (Join-Path $PSScriptRoot '../Cargo.toml'),
    [switch]$WriteGitHubOutput
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$releaseManifest = Get-Content -LiteralPath $ManifestPath -Raw
$releaseSection = [regex]::Match($releaseManifest, '(?ms)^\[workspace\.package\]\s*\r?\n(?<section>.*?)(?=^\[|\z)')
if (-not $releaseSection.Success) { throw 'Missing workspace.package / 缺少 workspace.package' }
$releaseVersions = [regex]::Matches($releaseSection.Groups['section'].Value, '(?m)^version\s*=\s*"(?<version>[^"\r\n]+)"\s*$')
if ($releaseVersions.Count -ne 1) { throw 'Expected one workspace version / 工作区必须只有一个版本号' }
$releaseVersion = $releaseVersions[0].Groups['version'].Value
# Numeric prerelease identifiers must not contain leading zeroes, as required by SemVer.
# 按 SemVer 要求，预发布版本中的纯数字标识不能包含前导零。
$releaseNumber = '(?:0|[1-9][0-9]*)'
$releaseIdentifier = '(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)'
$releaseSemVer = "$releaseNumber\.$releaseNumber\.$releaseNumber(?:-$releaseIdentifier(?:\.$releaseIdentifier)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
if ($releaseVersion -cnotmatch "\A$releaseSemVer\z") { throw 'Invalid SemVer version / 版本号不符合 SemVer' }
$releaseBranch = [regex]::Match($Ref, '\Arefs/heads/release/(?<version>[^/]+)\z')
if (-not $releaseBranch.Success -or $releaseBranch.Groups['version'].Value -cne $releaseVersion) {
    throw "Use release/$releaseVersion; branch and Cargo version must match / 仅允许与 Cargo 版本完全一致的版本分支"
}
$releasePrerelease = ($releaseVersion.Split('+')[0]).Contains('-')
if ($WriteGitHubOutput) {
    if (-not $env:GITHUB_OUTPUT) { throw 'GITHUB_OUTPUT is not set / 未设置 GITHUB_OUTPUT' }
    Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "version=$releaseVersion" -Encoding utf8
    Add-Content -LiteralPath $env:GITHUB_OUTPUT -Value "prerelease=$($releasePrerelease.ToString().ToLowerInvariant())" -Encoding utf8
}
[pscustomobject]@{ Version = $releaseVersion; Tag = "v$releaseVersion"; Prerelease = $releasePrerelease }
