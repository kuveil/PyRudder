# Opt-in, pinned compiler provisioning inside ignored target/build-tools, not a machine install.
# 显式将固定版本编译器准备到被忽略的 target/build-tools，不进行整机安装。
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$innoRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$innoTools = Join-Path $innoRepository 'target/build-tools'
$innoDirectory = Join-Path $innoTools 'inno-6.7.3'
$innoCompiler = Join-Path $innoDirectory 'ISCC.exe'
$innoCompilerHash = '0a8757031b33777e4c9cbffee40f11a5062b36d25cbe144c1db73b6102b80ad7'
if (Test-Path -LiteralPath $innoDirectory) {
    if (-not (Test-Path -LiteralPath $innoCompiler -PathType Leaf) -or
        (Get-FileHash -LiteralPath $innoCompiler -Algorithm SHA256).Hash -ne $innoCompilerHash) {
        throw 'Existing compiler directory is incomplete or unexpected; refusing overwrite / 已有编译器不完整或不匹配，拒绝覆盖'
    }
    Write-Output $innoCompiler
    return
}

[void](New-Item -ItemType Directory -Path $innoTools -Force)
$innoArchive = Join-Path $innoTools 'innosetup-6.7.3.exe'
if (-not (Test-Path -LiteralPath $innoArchive)) {
    Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $innoArchive
}
if ((Get-FileHash -LiteralPath $innoArchive -Algorithm SHA256).Hash -ne '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732') {
    throw 'Compiler package SHA256 mismatch / 编译器包 SHA256 不匹配'
}
$innoSignature = Get-AuthenticodeSignature -LiteralPath $innoArchive
if ($innoSignature.Status -ne 'Valid' -or $innoSignature.SignerCertificate.Subject -notmatch 'CN=Pyrsys B\.V\.,') {
    throw 'Compiler publisher/signature verification failed / 编译器发布者或签名验证失败'
}
# Official portable mode disables uninstall registration, associations and shortcuts.
# 官方便携模式不登记卸载项、文件关联和快捷方式。
$innoArguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/PORTABLE=1', '/TASKS=',
    ('/DIR="' + $innoDirectory + '"'), ('/LOG="' + (Join-Path $innoTools 'inno-portable.log') + '"'))
$innoProcess = Start-Process -FilePath $innoArchive -ArgumentList $innoArguments -WindowStyle Hidden -Wait -PassThru
if ($innoProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $innoCompiler)) {
    throw 'Portable compiler provisioning failed / 便携编译器准备失败'
}
if ((Get-FileHash -LiteralPath $innoCompiler -Algorithm SHA256).Hash -ne $innoCompilerHash) {
    throw 'Extracted compiler SHA256 mismatch / 解压后的编译器 SHA256 不匹配'
}
Write-Output $innoCompiler
