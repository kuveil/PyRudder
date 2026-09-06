# Offline tests of package privacy and stable dependency references.
# 离线测试发行包隐私保护与稳定依赖引用。
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../package-support.ps1')
$testRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ("../../target/package-validation/" + [Guid]::NewGuid().ToString('N'))))
[void](New-Item -ItemType Directory -Path $testRoot -Force)
$testUtf8 = [Text.UTF8Encoding]::new($false)
$testFile = Join-Path $testRoot 'fixture.txt'
$testCount = 0
function Assert-PackageRejected {
    param([scriptblock]$Action)
    $rejected = $false
    try { $null = & $Action } catch { $rejected = $true }
    if (-not $rejected) { throw 'Invalid package input accepted / 错误接受非法发行包输入' }
}
foreach ($privateText in @('C:\Users\builder\project\source.rs', 'C:/Users/builder/project/source.rs', 'C:\\Users\\builder\\project\\source.rs', 'path+file:///arbitrary/workspace')) {
    foreach ($encoding in @($testUtf8, [Text.Encoding]::Unicode)) {
        [IO.File]::WriteAllText($testFile, $privateText, $encoding)
        Assert-PackageRejected { Assert-PackagePrivacy -Directory $testRoot -PrivatePaths @('C:\Users\builder') }
        $testCount++
    }
}
[IO.File]::WriteAllText($testFile, '/pyrudder/crates/main.rs /cargo/registry/file.rs C:\Windows\System32\kernel32.dll', $testUtf8)
Assert-PackagePrivacy -Directory $testRoot -PrivatePaths @('C:\Users\builder')
$testCount++
[IO.File]::WriteAllBytes($testFile, ([byte[]]@(0x61) + [Text.Encoding]::Unicode.GetBytes('C:\Users\builder\source.rs')))
Assert-PackageRejected { Assert-PackagePrivacy -FilePaths @($testFile) -PrivatePaths @('C:\Users\builder') }
$testCount++
$flags = @(Get-PackageRemapFlags -Mappings ([ordered]@{ 'C:\Users\builder' = '/user'; 'C:\Users\builder\.cargo' = '/cargo'; 'E:\project' = '/pyrudder' }))
foreach ($flag in @('-C', 'target-feature=+crt-static', '--remap-path-prefix=C:/Users/builder/.cargo=/cargo', '--remap-path-prefix=C:\Users\builder\.cargo=/cargo', '--remap-path-prefix=E:/project=/pyrudder', '--remap-path-prefix=E:\project=/pyrudder')) {
    if ($flag -cnotin $flags) { throw 'Missing remapping flag / 缺少路径重映射参数' }
    $testCount++
}
$previousRef = $null
foreach ($workspacePath in @('E:/private/project', 'D:/different/project')) {
    $workspaceId = "path+file:///$workspacePath#app@1.0.0"
    $registryId = 'registry+https://github.com/rust-lang/crates.io-index#dependency@2.0.0'
    $metadata = [pscustomobject]@{
        packages = @(
            [pscustomobject]@{ id = $workspaceId; name = 'app'; version = '1.0.0'; source = $null; license = 'Apache-2.0' },
            [pscustomobject]@{ id = $registryId; name = 'dependency'; version = '2.0.0'; source = 'registry+https://github.com/rust-lang/crates.io-index'; license = 'MIT' }
        )
        resolve = [pscustomobject]@{ nodes = @(
            [pscustomobject]@{ id = $workspaceId; dependencies = @($registryId) },
            [pscustomobject]@{ id = $registryId; dependencies = @() }
        ) }
    }
    $bom = Get-PackageBom -Metadata $metadata -Version '1.0.0'
    $serialized = $bom | ConvertTo-Json -Depth 15
    if ($serialized.Contains($workspacePath) -or $serialized.Contains('path+file://')) { throw 'SBOM contains a workspace path / SBOM 含工作区路径' }
    $refs = @($bom.components | ForEach-Object { $_.'bom-ref' })
    foreach ($node in $bom.dependencies) {
        if ($node.ref -notin $refs) { throw 'Dangling node / 无效依赖节点' }
        foreach ($child in $node.dependsOn) { if ($child -notin $refs) { throw 'Dangling dependency / 无效依赖引用' } }
    }
    $appRef = ($bom.components | Where-Object { $_.name -eq 'app' }).'bom-ref'
    if ($previousRef -and $appRef -cne $previousRef) { throw 'Workspace location changes the public ID / 工作区位置改变了公开 ID' }
    $previousRef = $appRef
    $testCount++
}
$metadata.resolve.nodes[0].dependencies = @('missing-node')
Assert-PackageRejected { Get-PackageBom -Metadata $metadata -Version '1.0.0' }
$testCount++
# Keep the repository and packaged language navigation aligned.
# 保持仓库与发行包的语言导航一致。
$readmeRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$readmePackageScript = Get-Content -LiteralPath (Join-Path $readmeRepository 'scripts/package.ps1') -Raw -Encoding UTF8
$readmeInstallerScript = Get-Content -LiteralPath (Join-Path $readmeRepository 'scripts/installer/pyrudder.iss') -Raw -Encoding UTF8
$readmeReleaseScript = Get-Content -LiteralPath (Join-Path $readmeRepository 'scripts/release-publish.ps1') -Raw -Encoding UTF8
foreach ($readmeCase in @(
    @{ File = 'README.md'; Language = 'English'; Other = 'README_ZH.md'; Heading = 'Update an existing installation'; Anchor = 'update-an-existing-installation' },
    @{ File = 'README_ZH.md'; Language = '简体中文'; Other = 'README.md'; Heading = '更新已有安装'; Anchor = '更新已有安装' }
)) {
    $readmeText = Get-Content -LiteralPath (Join-Path $readmeRepository $readmeCase.File) -Raw -Encoding UTF8
    if (-not $readmeText.Contains("<strong>$($readmeCase.Language)</strong>") -or
        -not $readmeText.Contains(('href="./{0}"' -f $readmeCase.Other)) -or
        -not $readmeText.Contains("### $($readmeCase.Heading)")) {
        throw 'README language navigation is inconsistent / README 语言导航不一致'
    }
    if (-not $readmePackageScript.Contains("'$($readmeCase.File)'") -or
        -not $readmeInstallerScript.Contains(('Source: "{#PayloadDir}\' + $readmeCase.File + '"'))) {
        throw 'A translated README is missing from the distribution / 发行物缺少对应语言 README'
    }
    if (-not $readmeReleaseScript.Contains("/$($readmeCase.File)#$($readmeCase.Anchor)")) {
        throw 'Release guide points to the wrong README / 发布指南指向错误的 README'
    }
    $testCount++
}
Write-Output "Package validation passed: $testCount checks / 发行包验证通过：$testCount 项"
