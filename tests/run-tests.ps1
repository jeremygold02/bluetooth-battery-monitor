param(
    [string]$PopupScreenshot
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw "The .NET SDK is required to build the test suites."
}
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10"
$windowsMetadata = Get-ChildItem -LiteralPath (Join-Path $sdkRoot "UnionMetadata") -Filter "Windows.winmd" -Recurse -File |
    Sort-Object FullName -Descending | Select-Object -First 1
$referencesRoot = Join-Path $sdkRoot "References"
$universalContract = Get-ChildItem -LiteralPath $referencesRoot -Filter "Windows.Foundation.UniversalApiContract.winmd" -Recurse -File |
    Sort-Object FullName -Descending | Select-Object -First 1
$foundationContract = Get-ChildItem -LiteralPath $referencesRoot -Filter "Windows.Foundation.FoundationContract.winmd" -Recurse -File |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $windowsMetadata -or -not $universalContract -or -not $foundationContract) {
    throw "Windows SDK metadata required by the application was not found."
}

$suites = @(
    @{ File = "BatteryRateTracker.Tests.cs"; Main = "BatteryRateTrackerTests"; Arguments = @() },
    @{ File = "BatteryMonitorClient.Tests.cs"; Main = "BatteryMonitorClientTests"; Arguments = @() },
    @{ File = "LogitechHidBatteryCollector.Tests.cs"; Main = "LogitechHidBatteryCollectorTests"; Arguments = @() },
    @{ File = "TrayDevice.Tests.cs"; Main = "TrayDeviceTests"; Arguments = @() },
    @{ File = "Updater.Tests.cs"; Main = "UpdaterTests"; Arguments = @() },
    @{ File = "PopupUi.Tests.cs"; Main = "PopupUiTests"; Arguments = @($PopupScreenshot | Where-Object { $_ }) },
    @{ File = "SettingsUi.Tests.cs"; Main = "SettingsUiTests"; Arguments = @() },
    @{ File = "CodexUsageTray.Tests.cs"; Main = "CodexUsageTray.Tests.Program"; Arguments = @((Join-Path $PSScriptRoot "fixtures\rate-limits.json")) }
)
foreach ($suite in $suites) {
    $tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    $output = [System.IO.Path]::GetFullPath((Join-Path $tempRoot ("BluetoothBatteryTests-" + [guid]::NewGuid().ToString("N"))))
    if ([System.IO.Path]::GetDirectoryName($output) -ne $tempRoot) {
        throw "Test output directory is outside the temporary folder."
    }
    try {
        $arguments = @(
            "build", (Join-Path $projectRoot "BluetoothBatteryMonitor.csproj"),
            "--configuration", "Release", "--output", $output, "--verbosity", "quiet", "-t:Rebuild",
            "-p:TestSource=$(Join-Path $PSScriptRoot $suite.File)", "-p:TestMain=$($suite.Main)",
            "-p:WindowsMetadataPath=$($windowsMetadata.FullName)",
            "-p:UniversalContractPath=$($universalContract.FullName)",
            "-p:FoundationContractPath=$($foundationContract.FullName)"
        )
        & dotnet $arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Compilation failed for $($suite.File)."
        }
        & (Join-Path $output "BluetoothBatteryTests.exe") $suite.Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "Tests failed for $($suite.File)."
        }
    }
    finally {
        Remove-Item -LiteralPath $output -Recurse -Force -ErrorAction SilentlyContinue
    }
}
Write-Host "All battery monitor test suites passed."
