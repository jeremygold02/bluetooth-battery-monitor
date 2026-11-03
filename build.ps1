param(
    [switch]$Run
)

$ErrorActionPreference = "Stop"
$projectRoot = $PSScriptRoot
$project = Join-Path $projectRoot "BluetoothBatteryMonitor.csproj"
$output = Join-Path $projectRoot "BluetoothBatteryMonitor.exe"
$buildOutput = Join-Path $projectRoot "artifacts\build"

if (Get-Process -Name "BluetoothBatteryMonitor" -ErrorAction SilentlyContinue) {
    throw "BluetoothBatteryMonitor.exe is running. Exit it from the tray before rebuilding."
}
if (-not (Get-Command dotnet -ErrorAction SilentlyContinue)) {
    throw "The .NET SDK is required to build the application and restore its UI dependencies."
}
foreach ($requiredFile in @($project, (Join-Path $projectRoot "CodexUsageTray.cs"), (Join-Path $projectRoot "icon.ico"))) {
    if (-not (Test-Path -LiteralPath $requiredFile)) {
        throw "Required build input is missing: $requiredFile"
    }
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

& dotnet build $project --configuration Release --output $buildOutput `
    "-p:WindowsMetadataPath=$($windowsMetadata.FullName)" `
    "-p:UniversalContractPath=$($universalContract.FullName)" `
    "-p:FoundationContractPath=$($foundationContract.FullName)" --verbosity quiet
if ($LASTEXITCODE -ne 0) {
    throw "The application build failed."
}

$builtExecutable = Join-Path $buildOutput "BluetoothBatteryMonitor.exe"
if (-not (Test-Path -LiteralPath $builtExecutable)) {
    throw "Build output is missing: BluetoothBatteryMonitor.exe"
}
$embeddedNames = [Reflection.Assembly]::Load([IO.File]::ReadAllBytes($builtExecutable)).GetManifestResourceNames()
foreach ($name in @(
    "Wpf.Ui.dll", "Wpf.Ui.Abstractions.dll", "System.Memory.dll", "System.Buffers.dll",
    "System.Numerics.Vectors.dll", "System.Runtime.CompilerServices.Unsafe.dll"
)) {
    if ($embeddedNames -notcontains "BluetoothBatteryMonitor.Embedded.$name") {
        throw "Executable is missing embedded dependency: $name"
    }
}

# Distribute the UI toolkit's license and the notices for assets embedded in it.
$assets = Get-Content -LiteralPath (Join-Path $projectRoot "obj\project.assets.json") -Raw | ConvertFrom-Json
$uiPackage = $null
foreach ($packageRoot in $assets.packageFolders.PSObject.Properties.Name) {
    $candidate = Join-Path $packageRoot "wpf-ui\4.3.0"
    if (Test-Path -LiteralPath (Join-Path $candidate "LICENSE.md")) {
        $uiPackage = $candidate
        break
    }
}
if (-not $uiPackage) {
    throw "Could not find the restored WPF-UI package notices."
}
$noticesDirectory = Join-Path $buildOutput "licenses\WPF-UI"
New-Item -ItemType Directory -Path $noticesDirectory -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $uiPackage "LICENSE.md") -Destination (Join-Path $noticesDirectory "LICENSE.md") -Force
Copy-Item -LiteralPath (Join-Path $uiPackage "ThirdPartyNotices.txt") -Destination (Join-Path $noticesDirectory "ThirdPartyNotices.txt") -Force
Copy-Item -LiteralPath (Join-Path $projectRoot "THIRD_PARTY_NOTICES.md") -Destination (Join-Path $buildOutput "THIRD_PARTY_NOTICES.md") -Force

# The executable embeds its managed UI dependencies and can be updated as one file.
Copy-Item -LiteralPath $builtExecutable -Destination $output -Force
Write-Host "Built $output"
Write-Host "Complete redistributable files are in $buildOutput"

if ($Run) {
    Start-Process -FilePath $output -WindowStyle Hidden
}
