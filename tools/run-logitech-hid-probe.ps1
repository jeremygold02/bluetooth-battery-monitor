$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$source = Join-Path $scriptDir 'LogitechHidProbe.cs'
$exe = Join-Path $scriptDir 'LogitechHidProbe.exe'

$compilerCandidates = @(
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'),
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe')
)
$compiler = $compilerCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $compiler) {
    throw 'Could not find the .NET Framework C# compiler.'
}

& $compiler `
    /nologo `
    /target:exe `
    /optimize+ `
    /platform:anycpu `
    /out:$exe `
    /reference:System.dll `
    /reference:System.Core.dll `
    $source

if ($LASTEXITCODE -ne 0) {
    throw "Probe compilation failed with exit code $LASTEXITCODE."
}

& $exe
exit $LASTEXITCODE
