param(
    [Parameter(Mandatory = $true)][string]$Destination
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
$apacheTextCrate = 'serde'
$additionalFiles = @{ 'epaint_default_fonts' = 'fonts\*.txt' }

$json = (& cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $root 'Cargo.toml')) -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
$metadata = $json | ConvertFrom-Json

$packages = @{}
foreach ($package in $metadata.packages) { $packages[$package.id] = $package }
$nodes = @{}
foreach ($node in $metadata.resolve.nodes) { $nodes[$node.id] = $node }

$included = New-Object 'System.Collections.Generic.HashSet[string]'
$pending = New-Object 'System.Collections.Generic.Queue[string]'
$pending.Enqueue($metadata.resolve.root)
while ($pending.Count -gt 0) {
    $id = $pending.Dequeue()
    if (-not $included.Add($id)) { continue }
    foreach ($dependency in $nodes[$id].deps) {
        if (@($dependency.dep_kinds | Where-Object { $null -eq $_.kind }).Count -gt 0) { $pending.Enqueue($dependency.pkg) }
    }
}
[void]$included.Remove($metadata.resolve.root)
$crates = @($included | ForEach-Object { $packages[$_] } | Sort-Object name, version)

$licenses = Join-Path $Destination 'licenses'
if (Test-Path -LiteralPath $licenses) { Remove-Item -LiteralPath $licenses -Recurse -Force }
New-Item -ItemType Directory -Path $licenses -Force | Out-Null

$apacheSource = @($crates | Where-Object { $_.name -eq $apacheTextCrate } | Select-Object -First 1)
if ($apacheSource.Count -eq 0) { throw "The Apache License text source crate '$apacheTextCrate' is not a dependency." }
Copy-Item -LiteralPath (Join-Path (Split-Path -Parent $apacheSource[0].manifest_path) 'LICENSE-APACHE') -Destination (Join-Path $licenses 'Apache-2.0.txt')

foreach ($crate in $crates) {
    $source = Split-Path -Parent $crate.manifest_path
    $target = Join-Path $licenses "$($crate.name)-$($crate.version)"
    New-Item -ItemType Directory -Path $target | Out-Null
    $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(licen[sc]e|copying|notice|unlicense)' })
    foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination $target }
    if ($files.Count -eq 0) {
        if ("$($crate.license)" -notmatch 'Apache-2\.0') {
            throw "$($crate.name) $($crate.version) ships no license file and is not available under Apache-2.0."
        }
        $notice = @(
            "$($crate.name) $($crate.version) is licensed under $($crate.license).",
            'Its published package contains no license file. It is distributed here under the',
            'Apache License 2.0; a copy is in Apache-2.0.txt in the licenses folder.',
            "Source: $($crate.repository)"
        )
        Set-Content -LiteralPath (Join-Path $target 'LICENSE.txt') -Value $notice -Encoding UTF8
    }
    if ($additionalFiles.ContainsKey($crate.name)) {
        $pattern = $additionalFiles[$crate.name]
        $additional = @(Get-ChildItem -Path (Join-Path $source $pattern) -File)
        if ($additional.Count -eq 0) { throw "$($crate.name) $($crate.version) has no files matching $pattern." }
        $subfolder = Join-Path $target (Split-Path -Parent $pattern)
        New-Item -ItemType Directory -Path $subfolder -Force | Out-Null
        foreach ($file in $additional) { Copy-Item -LiteralPath $file.FullName -Destination $subfolder }
    }
}
$collected = Get-Date
foreach ($file in (Get-ChildItem -LiteralPath $licenses -Recurse -File)) { $file.LastWriteTime = $collected }

$table = @(
    '',
    '## Rust crates',
    '',
    'The app is built from these crates. Their license files are in the `licenses` folder, one folder per crate.',
    '',
    '| Crate | Version | License | Source |',
    '| --- | --- | --- | --- |'
) + @($crates | ForEach-Object { "| $($_.name) | $($_.version) | $($_.license) | $($_.repository) |" })
$notices = (Get-Content -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Raw).TrimEnd()
Set-Content -LiteralPath (Join-Path $Destination 'THIRD_PARTY_NOTICES.md') -Value ($notices + "`r`n" + ($table -join "`r`n")) -Encoding UTF8
Write-Host "Collected license notices for $($crates.Count) crates in $Destination."
