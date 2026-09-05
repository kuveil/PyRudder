# Dependency-free checks for notice collection and local-path privacy.
# 无外部测试依赖的许可证收集与本地路径隐私检查。
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../package-notices.ps1')

function Assert-NoticeTest {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

function Assert-NoticeFailure {
    param([scriptblock]$Action, [string]$Pattern)
    $noticeFailed = $false
    try { $null = & $Action } catch {
        $noticeFailed = $true
        Assert-NoticeTest ($_.Exception.Message -match $Pattern) 'The failure did not explain the missing or unsafe license.'
    }
    Assert-NoticeTest $noticeFailed 'Notice collection unexpectedly accepted incomplete or unsafe input.'
}

function Write-NoticeFixture {
    param([string]$Path, [string]$Text)
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

$noticeTestRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$noticeTestBase = [IO.Path]::GetFullPath((Join-Path $noticeTestRepository 'target/package-notices-tests'))
$noticeTestRoot = Join-Path $noticeTestBase ([guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($noticeTestRoot)
try {
    $noticeTestRust = Join-Path $noticeTestRoot 'rust'
    Write-NoticeFixture (Join-Path $noticeTestRust 'share/doc/rust/COPYRIGHT-library.html') '<h1>Library copyrights</h1><pre>Original &quot;terms&quot; &lt;holder&gt;</pre>'
    Write-NoticeFixture (Join-Path $noticeTestRust 'share/doc/rust/licenses/MIT.txt') 'MIT library terms'
    Write-NoticeFixture (Join-Path $noticeTestRust 'share/doc/rust/licenses/Apache-2.0.txt') 'Apache library terms'
    $noticeTestCrate = Join-Path $noticeTestRoot 'crate'
    Write-NoticeFixture (Join-Path $noticeTestCrate 'LICENSE') 'Original license terms preserved exactly.'
    Write-NoticeFixture (Join-Path $noticeTestCrate 'nested/NOTICE') 'Copyright Fixture Authors'
    Write-NoticeFixture (Join-Path $noticeTestCrate 'nested/third_party/LICENSE-MIT') 'Nested dependency terms'
    Write-NoticeFixture (Join-Path $noticeTestCrate 'licenses/additional.txt') 'Additional license terms'
    $noticeTestPackage = [pscustomobject]@{
        name = 'fixture-crate'; version = '1.0.0'; license = 'MIT'; license_file = $null
        manifest_path = Join-Path $noticeTestCrate 'Cargo.toml'; source = 'registry+https://github.com/rust-lang/crates.io-index'
    }
    $noticeTestMetadata = [pscustomobject]@{ packages = @($noticeTestPackage) }
    $noticeTestResult = Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust
    Assert-NoticeTest ($noticeTestResult -is [string]) 'Collection must return exactly one string.'
    Assert-NoticeTest ($noticeTestResult.Contains('Crate: fixture-crate 1.0.0')) 'Missing component identity.'
    Assert-NoticeTest ($noticeTestResult.Contains('Original license terms preserved exactly.')) 'The original terms were altered.'
    Assert-NoticeTest ($noticeTestResult.Contains('nested/third_party/LICENSE-MIT')) 'Nested third-party terms were omitted.'
    Assert-NoticeTest ($noticeTestResult.Contains('Source file: licenses/additional.txt')) 'License directory terms were omitted.'
    Assert-NoticeTest ($noticeTestResult.Contains('Original "terms" <holder>')) 'HTML conversion damaged literal legal text.'
    Assert-NoticeTest (-not $noticeTestResult.Contains($noticeTestRoot)) 'The output exposed an absolute build path.'

    $noticeTestPackage.license_file = 'licenses/additional.txt'
    $noticeTestExplicit = Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust
    Assert-NoticeTest (([regex]::Matches($noticeTestExplicit, 'Source file: licenses/additional\.txt')).Count -eq 1) 'Declared license files must not be duplicated.'
    $noticeTestPackage.license_file = '../rust/share/doc/rust/licenses/MIT.txt'
    Assert-NoticeFailure { Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust } 'outside its crate'
    $noticeTestPackage.license_file = $null

    $noticeTestEmpty = Join-Path $noticeTestRoot 'empty'
    [void][IO.Directory]::CreateDirectory($noticeTestEmpty)
    $noticeTestPackage.manifest_path = Join-Path $noticeTestEmpty 'Cargo.toml'
    Assert-NoticeFailure { Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust } 'No original license terms'
    Write-NoticeFixture (Join-Path $noticeTestEmpty 'NOTICE') 'Copyright alone is not a license grant.'
    Assert-NoticeFailure { Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust } 'No original license terms'
    Write-NoticeFixture (Join-Path $noticeTestEmpty 'LICENSE') ''
    Assert-NoticeFailure { Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestRust } 'empty'
    $noticeTestPackage.manifest_path = Join-Path $noticeTestCrate 'Cargo.toml'
    Assert-NoticeFailure { Get-PackageNotices -Metadata $noticeTestMetadata -Sysroot $noticeTestEmpty } 'standard-library copyright'

    # Exercise the actual lockfile too, including its exceptional notice layouts.
    # 同时检查实际锁文件，包括其特殊的许可证布局。
    Push-Location -LiteralPath $noticeTestRepository
    try {
        $noticeRealMetadata = cargo metadata --format-version 1 --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
        $noticeRealSysroot = rustc --print sysroot
        if ($LASTEXITCODE -ne 0) { throw 'rustc sysroot lookup failed.' }
    } finally { Pop-Location }
    $noticeRealResult = Get-PackageNotices -Metadata $noticeRealMetadata -Sysroot $noticeRealSysroot
    Assert-NoticeTest (([regex]::Matches($noticeRealResult, '(?m)^Crate: ')).Count -eq @($noticeRealMetadata.packages | Where-Object source).Count) 'At least one locked crate is missing.'
    Assert-NoticeTest ($noticeRealResult.Contains('src/polyfill/once_cell/LICENSE-MIT')) 'The ring embedded dependency license is missing.'
    Assert-NoticeTest ($noticeRealResult.Contains('third_party/fiat/LICENSE')) 'The fiat embedded dependency license is missing.'
    Assert-NoticeTest ($noticeRealResult.Contains('Redistribution option: MIT, as supplied in AUTHORS below.')) 'The r-efi license selection is missing.'
    Assert-NoticeTest (-not $noticeRealResult.Contains($noticeRealSysroot)) 'The real toolchain path leaked.'
    Assert-NoticeTest (-not $noticeRealResult.Contains($noticeTestRepository)) 'The real workspace path leaked.'
    foreach ($noticeRealPackage in $noticeRealMetadata.packages) {
        Assert-NoticeTest (-not $noticeRealResult.Contains((Split-Path -Parent $noticeRealPackage.manifest_path))) 'A crate source directory leaked.'
    }
    Write-Output 'Package notice tests passed, including all locked crates and Rust standard-library notices.'
} finally {
    # Remove only this run's owned fixture directory after validating its boundary.
    # 校验边界后仅删除本次运行创建的测试目录。
    $noticeTestCleanup = [IO.Path]::GetFullPath($noticeTestRoot)
    if (-not $noticeTestCleanup.StartsWith($noticeTestBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refusing to remove a test directory outside the intended test root.'
    }
    if (Test-Path -LiteralPath $noticeTestCleanup) { Remove-Item -LiteralPath $noticeTestCleanup -Recurse -Force }
}
