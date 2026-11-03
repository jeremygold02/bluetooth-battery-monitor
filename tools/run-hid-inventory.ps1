$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$source = Join-Path $scriptDir 'HidInventory.cs'
$exe = Join-Path $scriptDir 'HidInventory.exe'
$compiler = @(
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'),
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe')
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

if (-not $compiler) {
    throw 'Could not find the .NET Framework C# compiler.'
}

& $compiler /nologo /target:exe /optimize+ /platform:anycpu /out:$exe /reference:System.dll $source
if ($LASTEXITCODE -ne 0) {
    throw "Inventory compilation failed with exit code $LASTEXITCODE."
}

& $exe
exit $LASTEXITCODE
