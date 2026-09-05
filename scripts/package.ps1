# Build privacy-checked unsigned CLI distributions without installing them.
# 构建经过隐私检查的未签名 CLI 发行物，不执行安装。
[CmdletBinding()]
param(
    [string]$InnoCompiler,
    [switch]$ZipOnly,
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$packageRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'package-support.ps1')
. (Join-Path $PSScriptRoot 'package-notices.ps1')
$packagePreviousFlags = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
Push-Location -LiteralPath $packageRepository
try {
    $packageCommit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source revision / 无法确定源码版本' }
    $packageStatus = @(& git status --porcelain --untracked-files=normal)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect source status / 无法检查源码状态' }
    $packageDirty = $packageStatus.Count -gt 0
    if ($Release -and $packageDirty) { throw 'Release packaging requires a clean source tree / 正式打包要求源码工作区干净' }
    $packageProfile = [Environment]::GetFolderPath('UserProfile')
    $packageCargoRoot = if ($env:CARGO_HOME) { [IO.Path]::GetFullPath($env:CARGO_HOME) } else { Join-Path $packageProfile '.cargo' }
    $packageSysroot = (& rustc --print sysroot).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate Rust toolchain / 无法定位 Rust 工具链' }
    $packageMappings = [ordered]@{}
    $packageMappings[$packageProfile] = '/user'
    $packageMappings[$packageCargoRoot] = '/cargo'
    $packageMappings[$packageSysroot] = '/rust'
    $packageMappings[$packageRepository] = '/pyrudder'
    # Remapping is process-local and covers both Windows path separator forms.
    # 路径重映射仅作用于当前进程，并覆盖两种 Windows 路径分隔符。
    $env:CARGO_ENCODED_RUSTFLAGS = (Get-PackageRemapFlags -Mappings $packageMappings) -join [char]31
    # Require an explicitly provisioned compiler; never silently install build tools globally.
    # 要求显式准备编译器，绝不静默向全局安装构建工具。
    if (-not $ZipOnly) {
        if (-not $InnoCompiler) {
            $packageLocalCompiler = Join-Path $packageRepository 'target/build-tools/inno-6.7.3/ISCC.exe'
            if (Test-Path -LiteralPath $packageLocalCompiler) { $InnoCompiler = $packageLocalCompiler }
            else {
                $packageCompilerCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
                if ($packageCompilerCommand) { $InnoCompiler = $packageCompilerCommand.Source }
            }
        }
        if (-not $InnoCompiler -or -not (Test-Path -LiteralPath $InnoCompiler -PathType Leaf)) {
            throw 'Provide Inno Setup 6.7.3+ using -InnoCompiler <ISCC.exe>, or use -ZipOnly / 请指定 Inno 编译器，或使用 -ZipOnly'
        }
        $InnoCompiler = (Resolve-Path -LiteralPath $InnoCompiler).Path
        $packageCompilerHash = (Get-FileHash -LiteralPath $InnoCompiler -Algorithm SHA256).Hash.ToLowerInvariant()
        # ISCC's PE version is 0.0.0.0; identify the pinned tool by its verified bytes.
        # ISCC 的 PE 版本是占位值 0.0.0.0；通过已验证字节标识固定工具。
        $packageCompilerInfo = [ordered]@{
            name = 'Inno Setup'
            version = if ($packageCompilerHash -eq '0a8757031b33777e4c9cbffee40f11a5062b36d25cbe144c1db73b6102b80ad7') { '6.7.3' } else { 'external compiler; inspect its distribution' }
            sha256 = $packageCompilerHash
        }
    }
    & cargo build --release --locked --target x86_64-pc-windows-msvc -p pyrudder-cli -p pyrudder-shim-console -p pyrudder-shim-gui
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed / 发布构建失败' }
    $packageMetadataText = & cargo metadata --locked --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Dependency inventory failed / 依赖清单失败' }
    $packageMetadata = ($packageMetadataText -join "`n") | ConvertFrom-Json
    $packageCli = $packageMetadata.packages | Where-Object name -eq 'pyrudder-cli' | Select-Object -First 1
    $packageVersion = $packageCli.version
    $packageStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $packageName = if ($Release) { "pyrudder-$packageVersion-windows-x64" } else { "pyrudder-$packageVersion-windows-x64-$packageStamp" }
    $packageDist = Join-Path $packageRepository 'target/dist'
    $packageDirectory = Join-Path $packageRepository "target/package-staging/$packageName"
    $packageZip = Join-Path $packageDist "$packageName.zip"
    if ((Test-Path -LiteralPath $packageDirectory) -or (Test-Path -LiteralPath $packageZip) -or (Test-Path -LiteralPath (Join-Path $packageDist "$packageName-Setup.exe"))) { throw 'Refusing to overwrite an earlier package / 拒绝覆盖已有发行包' }
    [void](New-Item -ItemType Directory -Path $packageDist -Force)
    [void](New-Item -ItemType Directory -Path (Join-Path $packageDirectory 'bin'))
    foreach ($packageBinary in @('pyrudder.exe', 'pyrudder-shim-console.exe', 'pyrudder-shim-gui.exe')) {
        $packageInput = Join-Path $packageRepository "target/x86_64-pc-windows-msvc/release/$packageBinary"
        Copy-Item -LiteralPath $packageInput -Destination (Join-Path $packageDirectory "bin/$packageBinary")
    }
    Copy-Item -LiteralPath (Join-Path $packageRepository 'scripts/distribution/pyrudder-layout.json') -Destination (Join-Path $packageDirectory 'bin')
    foreach ($packageReadme in @('README.md', 'README_EN.md', 'LICENSE', 'NOTICE')) {
        Copy-Item -LiteralPath (Join-Path $packageRepository $packageReadme) -Destination $packageDirectory
    }
    $packageAssets = Join-Path $packageDirectory 'assets'
    [void](New-Item -ItemType Directory -Path $packageAssets)
    foreach ($packageAsset in @('pyrudder-logo.svg', 'pyrudder-logo.png', 'banner.txt')) {
        Copy-Item -LiteralPath (Join-Path $packageRepository "assets/$packageAsset") -Destination $packageAssets
    }
    $packageBuildInfo = [ordered]@{
        name = 'PyRudder'; version = $packageVersion; target = 'x86_64-pc-windows-msvc'
        built_at_utc = [DateTime]::UtcNow.ToString('o'); source_commit = $packageCommit
        source_dirty = $packageDirty; rustc = (& rustc --version); static_crt = $true
        signed = $false
        layout = 'in-place root with bin/config/shims and managed data siblings'
        installer_compiler = if ($ZipOnly) { $null } else { $packageCompilerInfo }
        distribution = 'Unsigned Windows CLI distribution'
        license = 'Apache-2.0'
    }
    $packageUtf8 = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText((Join-Path $packageDirectory 'BUILD-INFO.json'), ($packageBuildInfo | ConvertTo-Json -Depth 6), $packageUtf8)
    $packageNotices = Get-PackageNotices -Metadata $packageMetadata -Sysroot $packageSysroot
    [IO.File]::WriteAllText((Join-Path $packageDirectory 'THIRD-PARTY-NOTICES.txt'), $packageNotices, $packageUtf8)
    $packageBom = Get-PackageBom -Metadata $packageMetadata -Version $packageVersion
    [IO.File]::WriteAllText((Join-Path $packageDirectory 'SBOM.cdx.json'), ($packageBom | ConvertTo-Json -Depth 15), $packageUtf8)
    Assert-PackagePrivacy -Directory $packageDirectory -PrivatePaths @($packageMappings.Keys)
    $packageChecksums = @(Get-ChildItem -LiteralPath $packageDirectory -File -Recurse | Sort-Object FullName | ForEach-Object {
        $packageRelative = $_.FullName.Substring($packageDirectory.Length + 1).Replace('\', '/')
        "$( (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() )  $packageRelative"
    })
    [IO.File]::WriteAllLines((Join-Path $packageDirectory 'SHA256SUMS.txt'), $packageChecksums, $packageUtf8)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::CreateFromDirectory($packageDirectory, $packageZip, [IO.Compression.CompressionLevel]::Optimal, $false)
    $packageZipHash = (Get-FileHash -LiteralPath $packageZip -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText("$packageZip.sha256", "$packageZipHash  $packageName.zip`n", $packageUtf8)
    # Verify archive entries against hashes without running any packaged application.
    # 通过摘要检查归档条目，不运行任何已打包应用。
    $packageOpened = [IO.Compression.ZipFile]::OpenRead($packageZip)
    try {
        foreach ($packageLine in $packageChecksums) {
            $packageExpected = $packageLine.Substring(0, 64)
            $packageEntryName = $packageLine.Substring(66)
            $packageEntry = $packageOpened.GetEntry($packageEntryName)
            if (-not $packageEntry) { throw "Missing archive entry / 归档缺少条目: $packageEntryName" }
            $packageStream = $packageEntry.Open()
            $packageHashAlgorithm = [Security.Cryptography.SHA256]::Create()
            try { $packageActual = [BitConverter]::ToString($packageHashAlgorithm.ComputeHash($packageStream)).Replace('-', '').ToLowerInvariant() }
            finally { $packageHashAlgorithm.Dispose(); $packageStream.Dispose() }
            if ($packageExpected -ne $packageActual) { throw "Archive hash mismatch / 归档摘要不一致: $packageEntryName" }
        }
    } finally { $packageOpened.Dispose() }
    Write-Output "Package / 发行包: $packageZip"
    Write-Output "SHA256: $packageZipHash"
    if (-not $ZipOnly) {
        $packageSetupName = "$packageName-Setup"
        & $InnoCompiler '/Qp' "/DPayloadDir=$packageDirectory" "/DPackageVersion=$packageVersion" "/DDistDir=$packageDist" "/DOutputName=$packageSetupName" (Join-Path $packageRepository 'scripts/installer/pyrudder.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed / 安装包编译失败' }
        $packageSetup = Join-Path $packageDist "$packageSetupName.exe"
        Assert-PackagePrivacy -FilePaths @($packageSetup) -PrivatePaths @($packageMappings.Keys)
        $packageSetupHash = (Get-FileHash -LiteralPath $packageSetup -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText("$packageSetup.sha256", "$packageSetupHash  $packageSetupName.exe`n", $packageUtf8)
        Write-Output "Installer / 安装包: $packageSetup"
        Write-Output "SHA256: $packageSetupHash"
    }
    Write-Output 'Package privacy and archive integrity checks passed. / 发行包路径隐私与归档完整性检查通过。'
} finally {
    [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $packagePreviousFlags, 'Process')
    Pop-Location
}
