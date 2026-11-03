$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$source = Join-Path $scriptDir 'HidppQuery.cs'
$exe = Join-Path $scriptDir 'HidppQuery.exe'
$compiler = @(
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'),
    (Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe')
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1

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
    throw "HID++ query compilation failed with exit code $LASTEXITCODE."
}

$path = '\\?\hid#vid_046d&pid_c53f&mi_02&col02#7&156f5e46&0&0001#{4d1e55b2-f16f-11cf-88cb-001111000030}'
& $exe $path
exit $LASTEXITCODE
