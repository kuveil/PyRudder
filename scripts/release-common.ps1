# Reusable validation for release assets; these functions never change GitHub state.
# 发布附件的可复用校验；这些函数不会修改 GitHub 状态。
Set-StrictMode -Version Latest

function Get-ReleaseAssetNames {
    param([Parameter(Mandatory)][string]$Version)
    $releaseAssetStem = "pyrudder-$Version-windows-x64"
    @("$releaseAssetStem.zip", "$releaseAssetStem.zip.sha256", "$releaseAssetStem-Setup.exe", "$releaseAssetStem-Setup.exe.sha256")
}

function Assert-ReleaseChecksum {
    param([Parameter(Mandatory)][string]$PayloadPath, [Parameter(Mandatory)][string]$ChecksumPath)
    $releaseChecksumName = [IO.Path]::GetFileName($PayloadPath)
    $releaseChecksumHash = (Get-FileHash -LiteralPath $PayloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $releaseChecksumText = [IO.File]::ReadAllText($ChecksumPath).TrimEnd([char[]]"`r`n")
    if ($releaseChecksumText -cne "$releaseChecksumHash  $releaseChecksumName") {
        throw "Release checksum mismatch / 发布摘要不匹配: $releaseChecksumName"
    }
}

function Assert-ReleaseArchive {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Version, [Parameter(Mandatory)][string]$Commit)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $releaseArchive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $releaseBuildEntry = $releaseArchive.GetEntry('BUILD-INFO.json')
        if (-not $releaseBuildEntry -or $releaseBuildEntry.Length -gt 65536) {
            throw 'Missing or oversized build information / 构建信息缺失或过大'
        }
        $releaseBuildReader = [IO.StreamReader]::new($releaseBuildEntry.Open())
        try { $releaseBuild = $releaseBuildReader.ReadToEnd() | ConvertFrom-Json }
        finally { $releaseBuildReader.Dispose() }
        if ($releaseBuild.name -cne 'PyRudder' -or $releaseBuild.version -cne $Version -or
            $releaseBuild.source_commit -cne $Commit -or $releaseBuild.source_dirty -isnot [bool] -or $releaseBuild.source_dirty) {
            throw 'Package source/version mismatch or dirty source / 包版本或提交不匹配，或源码存在未提交修改'
        }
    } finally { $releaseArchive.Dispose() }
}

function Assert-ReleaseAssetSet {
    param([Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Assets, [Parameter(Mandatory)][string[]]$ExpectedNames, [switch]$Complete)
    $releaseSeenNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($releaseAsset in $Assets) {
        if ($ExpectedNames -cnotcontains $releaseAsset.name -or -not $releaseSeenNames.Add($releaseAsset.name)) {
            throw 'Unexpected or duplicate release assets; refusing to alter the release / 发布附件意外或重复，拒绝修改'
        }
    }
    if ($Complete -and $releaseSeenNames.Count -ne $ExpectedNames.Count) {
        throw 'Published release is incomplete; refusing to change published assets / 已公开发布的附件不完整，拒绝修改已公开附件'
    }
}

function Assert-ReleaseIdentity {
    param([Parameter(Mandatory)][object]$Release, [Parameter(Mandatory)][string]$Tag, [Parameter(Mandatory)][string]$Commit)
    foreach ($releaseProperty in @('tag_name', 'target_commitish', 'draft', 'prerelease', 'assets')) {
        if ($Release.PSObject.Properties.Name -notcontains $releaseProperty) {
            throw 'Release response is missing required state / 发布响应缺少必要状态'
        }
    }
    if ($Release.draft -isnot [bool] -or $Release.prerelease -isnot [bool] -or $Release.assets -isnot [array]) {
        throw 'Release response has invalid state types / 发布响应的状态类型错误'
    }
    if ($Release.tag_name -cne $Tag -or $Release.target_commitish -cne $Commit) {
        throw 'This version already belongs to another source revision; increment the version / 此版本已属于其他源码提交，请提升版本号'
    }
}
