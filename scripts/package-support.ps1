# Shared reproducible references and build-path privacy checks.
# 共享稳定依赖引用与构建路径隐私检查。
Set-StrictMode -Version Latest

function Get-PackageRemapFlags {
    param([Parameter(Mandatory)][System.Collections.IDictionary]$Mappings)
    @('-C', 'target-feature=+crt-static')
    foreach ($entry in $Mappings.GetEnumerator()) {
        foreach ($source in @(([string]$entry.Key).Replace('\', '/'), ([string]$entry.Key).Replace('/', '\')) | Select-Object -Unique) {
            "--remap-path-prefix=$source=$($entry.Value)"
        }
    }
}

function Get-PackageBom {
    param([Parameter(Mandatory)]$Metadata, [Parameter(Mandatory)][string]$Version)
    $references = @{}
    $components = @($Metadata.packages | Sort-Object name, version, source | ForEach-Object {
        # Cargo workspace IDs contain absolute paths; never serialize those identifiers.
        # Cargo 工作区 ID 包含绝对路径，禁止直接序列化这些标识。
        $source = if ($_.source) { [string]$_.source } else { 'workspace' }
        $identity = "$($_.name)`n$($_.version)`n$source"
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try { $digest = [BitConverter]::ToString($algorithm.ComputeHash([Text.Encoding]::UTF8.GetBytes($identity))).Replace('-', '').ToLowerInvariant() }
        finally { $algorithm.Dispose() }
        $reference = "urn:pyrudder:component:$digest"
        if ($references.ContainsValue($reference)) { throw 'Duplicate package identity / 依赖身份重复' }
        $references[$_.id] = $reference
        $component = [ordered]@{
            type = 'library'; 'bom-ref' = $reference; name = $_.name; version = $_.version
            purl = "pkg:cargo/$($_.name)@$([Uri]::EscapeDataString($_.version))"
        }
        if ($_.license) { $component.licenses = @(@{ expression = $_.license }) }
        $component
    })
    $dependencies = @($Metadata.resolve.nodes | ForEach-Object {
        if (-not $references.ContainsKey($_.id)) { throw 'Unknown dependency node / 未知依赖节点' }
        $children = @($_.dependencies | ForEach-Object {
            if (-not $references.ContainsKey($_)) { throw 'Unknown dependency reference / 未知依赖引用' }
            $references[$_]
        })
        @{ ref = $references[$_.id]; dependsOn = $children }
    })
    [ordered]@{
        bomFormat = 'CycloneDX'; specVersion = '1.6'; version = 1
        metadata = @{
            timestamp = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
            component = @{ type = 'application'; name = 'PyRudder'; version = $Version; licenses = @(@{ license = @{ id = 'Apache-2.0' } }) }
            properties = @(@{ name = 'pyrudder:inventory-scope'; value = 'Complete locked workspace dependency graph, including build, development and non-Windows dependencies; not a binary reachability audit' })
        }
        components = $components
        dependencies = $dependencies
    }
}

function Assert-PackagePrivacy {
    [CmdletBinding(DefaultParameterSetName = 'Directory')]
    param(
        [Parameter(Mandatory, ParameterSetName = 'Directory')][string]$Directory,
        [Parameter(Mandatory, ParameterSetName = 'Files')][string[]]$FilePaths,
        [Parameter(Mandatory)][string[]]$PrivatePaths
    )
    $needles = @($PrivatePaths | Where-Object { $_ } | ForEach-Object {
        $_.TrimEnd('\', '/').Replace('\', '/')
        $_.TrimEnd('\', '/').Replace('/', '\')
        $_.TrimEnd('\', '/').Replace('/', '\').Replace('\', '\\')
    } | Select-Object -Unique)
    $files = if ($PSCmdlet.ParameterSetName -eq 'Directory') { Get-ChildItem -LiteralPath $Directory -File -Recurse } else { Get-Item -LiteralPath $FilePaths }
    foreach ($file in $files) {
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        # Binary strings need not begin at an even byte offset.
        # 二进制中的字符串不一定从偶数字节偏移开始。
        $representations = @([Text.Encoding]::UTF8.GetString($bytes), [Text.Encoding]::Unicode.GetString($bytes))
        if ($bytes.Length -gt 1) { $representations += [Text.Encoding]::Unicode.GetString($bytes, 1, $bytes.Length - 1) }
        foreach ($text in $representations) {
            $leaked = $text.IndexOf('path+file://', [StringComparison]::OrdinalIgnoreCase) -ge 0
            foreach ($needle in $needles) {
                if ($text.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) { $leaked = $true; break }
            }
            if ($leaked) {
                # Report only the affected artifact name, never the private value itself.
                # 仅报告受影响文件名，绝不在错误中再次输出隐私内容。
                throw "Private build path detected in artifact / 发行物中检测到私有构建路径: $($file.Name)"
            }
        }
    }
}
