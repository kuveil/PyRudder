# Collect original license texts without publishing build-machine paths.
# 收集原始许可证文本，不公开构建机器路径。
function Get-PackageNoticeFiles {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Root)

    $noticeDirectories = [Collections.Generic.Stack[string]]::new()
    $noticeDirectories.Push($Root)
    while ($noticeDirectories.Count -gt 0) {
        $noticeDirectory = $noticeDirectories.Pop()
        foreach ($noticeEntry in Get-ChildItem -LiteralPath $noticeDirectory -Force) {
            # Do not follow package symlinks into unrelated local files.
            # 不跟随包内符号链接读取无关本地文件。
            if (($noticeEntry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw 'A dependency contains a linked file or directory; review its notices before packaging.'
            }
            if ($noticeEntry.PSIsContainer) {
                $noticeDirectories.Push($noticeEntry.FullName)
            } else {
                $noticeRelative = $noticeEntry.FullName.Substring($Root.Length + 1).Replace('\', '/')
                if ($noticeEntry.Name -match '^(licen[cs]e|copying|copyright|notice|authors)([-._].*)?$' -or
                    ($noticeRelative -match '(^|/)licen[cs]es/' -and
                     $noticeEntry.Extension -in @('', '.txt', '.md', '.rst', '.html'))) {
                    $noticeEntry
                }
            }
        }
    }
}

function Read-PackageNoticeText {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)

    $noticeText = [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::new($false, $true))
    if ([string]::IsNullOrWhiteSpace($noticeText) -or $noticeText.Contains([char]0)) {
        throw 'A dependency license is empty or is not a text document.'
    }
    if ([IO.Path]::GetExtension($Path) -eq '.html') {
        # Strip markup before decoding entities so literal legal placeholders survive.
        # 先移除标记再解码实体，保留法律文本中的字面占位内容。
        $noticeText = [Net.WebUtility]::HtmlDecode([regex]::Replace($noticeText, '<[^>]+>', ''))
    }
    return $noticeText.Replace("`r`n", "`n").TrimEnd()
}

function Get-PackageNotices {
    [CmdletBinding()]
    [OutputType([string])]
    param(
        [Parameter(Mandatory)][object]$Metadata,
        [Parameter(Mandatory)][string]$Sysroot
    )

    $noticeOutput = [Text.StringBuilder]::new()
    [void]$noticeOutput.AppendLine('THIRD-PARTY NOTICES')
    [void]$noticeOutput.AppendLine('This distribution includes software from the Rust project and third-party crates.')
    [void]$noticeOutput.AppendLine('The original notices and license terms below remain applicable to their respective components.')
    [void]$noticeOutput.AppendLine('The crate inventory covers the lockfile, including target-specific and build dependencies; not every component is linked into every executable.')
    [void]$noticeOutput.AppendLine('A declared SPDX expression is descriptive metadata, not a replacement for the reproduced terms.')
    [void]$noticeOutput.AppendLine()

    $noticePackages = @($Metadata.packages | Where-Object { $_.source } | Sort-Object name, version)
    if ($noticePackages.Count -eq 0) { throw 'No third-party packages were provided for notice collection.' }
    foreach ($noticePackage in $noticePackages) {
        $noticeIdentity = "$($noticePackage.name) $($noticePackage.version)"
        if ($noticeIdentity -match '[\r\n]' -or [string]::IsNullOrWhiteSpace($noticePackage.license)) {
            throw 'A dependency is missing valid name/version/license metadata; review it before packaging.'
        }
        $noticeRoot = [IO.Path]::GetFullPath((Split-Path -Parent $noticePackage.manifest_path)).TrimEnd('\', '/')
        if (-not (Test-Path -LiteralPath $noticeRoot -PathType Container)) {
            throw "Source files are unavailable for $noticeIdentity; run cargo fetch --locked before packaging."
        }
        $noticeFiles = [Collections.Generic.SortedDictionary[string, string]]::new([StringComparer]::Ordinal)
        foreach ($noticeFile in Get-PackageNoticeFiles -Root $noticeRoot) {
            $noticeRelative = $noticeFile.FullName.Substring($noticeRoot.Length + 1).Replace('\', '/')
            $noticeFiles[$noticeRelative] = $noticeFile.FullName
        }
        if ($noticePackage.license_file) {
            $noticeExplicit = if ([IO.Path]::IsPathRooted($noticePackage.license_file)) {
                [IO.Path]::GetFullPath($noticePackage.license_file)
            } else {
                [IO.Path]::GetFullPath((Join-Path $noticeRoot $noticePackage.license_file))
            }
            if (-not $noticeExplicit.StartsWith($noticeRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
                -not (Test-Path -LiteralPath $noticeExplicit -PathType Leaf)) {
                throw "The declared license file for $noticeIdentity is missing or outside its crate."
            }
            $noticeFiles[$noticeExplicit.Substring($noticeRoot.Length + 1).Replace('\', '/')] = $noticeExplicit
        }
        $noticeHasTerms = @($noticeFiles.Keys | Where-Object {
            [IO.Path]::GetFileName($_) -match '^(licen[cs]e|copying)([-._].*)?$' -or $_ -match '(^|/)licen[cs]es/'
        }).Count -gt 0 -or [bool]$noticePackage.license_file

        # Some crates place the complete MIT terms in COPYRIGHT rather than LICENSE.
        # 部分 crate 将完整 MIT 条款放在 COPYRIGHT 中，而非 LICENSE。
        if (-not $noticeHasTerms -and $noticePackage.license -match '(^|[ (])MIT([ )]|$)' -and
            $noticeFiles.ContainsKey('COPYRIGHT')) {
            $noticeCopyright = Read-PackageNoticeText -Path $noticeFiles['COPYRIGHT']
            $noticeHasTerms = $noticeCopyright -match 'MIT License' -and
                $noticeCopyright -match 'Permission is hereby granted' -and
                $noticeCopyright -match 'THE SOFTWARE IS PROVIDED'
        }

        # r-efi publishes its complete MIT grant and copyright in AUTHORS, not LICENSE.
        # r-efi 将完整 MIT 授权和版权放在 AUTHORS 中，而非 LICENSE。
        $noticeAuthorsMit = $false
        if (-not $noticeHasTerms -and $noticePackage.name -eq 'r-efi' -and $noticePackage.version -eq '6.0.0' -and
            $noticePackage.license -match '(^|[ (])MIT([ )]|$)' -and $noticeFiles.ContainsKey('AUTHORS')) {
            $noticeAuthors = Read-PackageNoticeText -Path $noticeFiles['AUTHORS']
            $noticeAuthorsMit = $noticeAuthors -match 'AUTHORS-MIT:' -and
                $noticeAuthors -match 'Permission is hereby granted' -and $noticeAuthors -match 'COPYRIGHT:'
            $noticeHasTerms = $noticeAuthorsMit
        }
        if (-not $noticeHasTerms) {
            throw "No original license terms were found for $noticeIdentity; review and add an explicit collection rule before publishing."
        }
        [void]$noticeOutput.AppendLine(('=' * 78))
        [void]$noticeOutput.AppendLine("Crate: $noticeIdentity")
        [void]$noticeOutput.AppendLine("Declared license: $($noticePackage.license)")
        if ($noticeAuthorsMit) { [void]$noticeOutput.AppendLine('Redistribution option: MIT, as supplied in AUTHORS below.') }
        foreach ($noticeEntry in $noticeFiles.GetEnumerator()) {
            [void]$noticeOutput.AppendLine()
            [void]$noticeOutput.AppendLine("--- Source file: $($noticeEntry.Key) ---")
            [void]$noticeOutput.AppendLine((Read-PackageNoticeText -Path $noticeEntry.Value))
        }
        [void]$noticeOutput.AppendLine()
    }

    # Preserve the installed standard library's own inventory, not the compiler's unrelated components.
    # 保留已安装标准库自身的清单，不混入编译器的无关组件。
    $noticeRustRoot = [IO.Path]::GetFullPath($Sysroot).TrimEnd('\', '/')
    $noticeRustCopyright = 'share/doc/rust/COPYRIGHT-library.html'
    if (-not (Test-Path -LiteralPath (Join-Path $noticeRustRoot $noticeRustCopyright) -PathType Leaf)) {
        throw 'Rust standard-library copyright notices are missing; install the matching rust-docs component before packaging.'
    }
    $noticeRustFiles = @($noticeRustCopyright, 'share/doc/rust/licenses/MIT.txt', 'share/doc/rust/licenses/Apache-2.0.txt')
    foreach ($noticeRustFile in $noticeRustFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $noticeRustRoot $noticeRustFile) -PathType Leaf)) {
            throw 'The Rust toolchain is missing original standard-library license texts; install its matching rust-docs component.'
        }
    }
    [void]$noticeOutput.AppendLine(('=' * 78))
    [void]$noticeOutput.AppendLine('Component: Rust standard library')
    [void]$noticeOutput.AppendLine('Declared license: Apache-2.0 OR MIT, with the exceptions documented below.')
    [void]$noticeOutput.AppendLine('HTML documents are reproduced as text with markup removed and character entities decoded.')
    foreach ($noticeRustFile in $noticeRustFiles) {
        [void]$noticeOutput.AppendLine()
        [void]$noticeOutput.AppendLine("--- Source file: $noticeRustFile ---")
        [void]$noticeOutput.AppendLine((Read-PackageNoticeText -Path (Join-Path $noticeRustRoot $noticeRustFile)))
    }
    return $noticeOutput.ToString().Replace("`r`n", "`n")
}
