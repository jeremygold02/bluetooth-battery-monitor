param(
    [Parameter(Mandatory = $true)][string]$Bridge,
    [Parameter(Mandatory = $true)][string]$NativeExecutable,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appName = 'BluetoothBatteryMonitor.exe'
$restartArguments = [ordered]@{
    '0.1.0' = ''
    '0.1.7' = '--skip-startup-update --update-failed'
}
$Bridge = (Resolve-Path -LiteralPath $Bridge).Path
$NativeExecutable = (Resolve-Path -LiteralPath $NativeExecutable).Path
$bridgeProject = Join-Path (Split-Path -Parent $PSScriptRoot) 'BluetoothBatteryMonitor.csproj'
$hostProject = Join-Path $PSScriptRoot 'LegacyHost.csproj'
$workRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$work = Join-Path $workRoot ('bbm-migration-' + [guid]::NewGuid().ToString('N'))

function Invoke-Build {
    param([string]$Project, [string]$Artifacts, [string[]]$Properties = @())
    & dotnet build $Project -c Release --nologo -v quiet --disable-build-servers --artifacts-path $Artifacts @Properties | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Build failed: $Project" }
}

function Get-Sha256 {
    param([string]$Path)
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Get-SandboxProcesses {
    param([string]$Directory)
    $prefix = $Directory.TrimEnd('\') + '\'
    @(Get-CimInstance Win32_Process | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
    })
}

function Stop-SandboxProcesses {
    param([string]$Directory)
    foreach ($process in (Get-SandboxProcesses $Directory)) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

function Wait-Until {
    param([scriptblock]$Condition, [int]$Seconds, [string]$Failure)
    $deadline = (Get-Date).AddSeconds($Seconds)
    while (-not (& $Condition)) {
        if ((Get-Date) -gt $deadline) { throw $Failure }
        Start-Sleep -Milliseconds 250
    }
}

function New-Sandbox {
    param([string]$Name, [string]$Installed)
    $root = Join-Path $work $Name
    $sandbox = [pscustomobject]@{
        App     = Join-Path $root 'app'
        AppData = Join-Path $root 'appdata'
        Temp    = Join-Path $root 'temp'
        Target  = Join-Path (Join-Path $root 'app') $appName
    }
    foreach ($directory in @($sandbox.App, $sandbox.AppData, $sandbox.Temp)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    Copy-Item -LiteralPath $Installed -Destination $sandbox.Target
    $sandbox
}

function Start-Sandboxed {
    param($Sandbox, [string[]]$Arguments)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Sandbox.Target
    $startInfo.Arguments = ($Arguments | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $startInfo.WorkingDirectory = $Sandbox.App
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables['APPDATA'] = $Sandbox.AppData
    $startInfo.EnvironmentVariables['TEMP'] = $Sandbox.Temp
    $startInfo.EnvironmentVariables['TMP'] = $Sandbox.Temp
    [System.Diagnostics.Process]::Start($startInfo)
}

function Get-UpdateLog {
    param([string]$UpdateDirectory)
    $log = Join-Path $UpdateDirectory 'update.log'
    if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Raw } else { '' }
}

function Invoke-LegacyUpdate {
    param([string]$Generation, $Sandbox, [string]$Asset)
    $process = Start-Sandboxed $Sandbox @('--install', $Asset, $Version)
    if (-not $process.WaitForExit(60000)) { throw "The $Generation updater did not finish preparing the update." }
    if ($process.ExitCode -ne 0) { throw "The $Generation updater rejected the update (exit code $($process.ExitCode))." }
    $updates = @(Get-ChildItem -LiteralPath $Sandbox.Temp -Directory -Filter 'bluetooth_battery_monitor_update_*')
    if ($updates.Count -ne 1) { throw "The $Generation updater did not create its update folder." }
    $updateScript = Join-Path $updates[0].FullName 'update.ps1'
    Wait-Until { -not (Test-Path -LiteralPath $updateScript) } 120 "The $Generation update script did not finish."
    Start-Sleep -Seconds 1
    $updates[0].FullName
}

function Test-AssetSelection {
    param([string]$Generation, [string]$LegacyHost)
    $selected = & $LegacyHost --select 'BluetoothBatteryMonitorRust.exe' 'BluetoothBatteryMonitor-Setup.exe' 'ThirdPartyNotices.zip' $appName
    if ($LASTEXITCODE -ne 0 -or "$selected".Trim() -ne $appName) {
        throw "The $Generation updater selected '$selected' instead of $appName."
    }
}

function Test-Migration {
    param([string]$Generation, [string]$LegacyHost)
    $sandbox = New-Sandbox "migration-$Generation" $LegacyHost
    $updateDirectory = Invoke-LegacyUpdate $Generation $sandbox $Bridge
    if (Test-Path -LiteralPath $updateDirectory) {
        throw "The $Generation updater did not complete the update:`n$(Get-UpdateLog $updateDirectory)"
    }
    if (Test-Path -LiteralPath ($sandbox.Target + '.restored')) { throw "The $Generation updater rolled the update back." }
    if ((Get-Sha256 $sandbox.Target) -ne (Get-Sha256 $NativeExecutable)) {
        throw "The installed file is not the native app after the $Generation update."
    }
    Wait-Until { (Get-SandboxProcesses $sandbox.App).Count -le 1 } 20 "The bridge did not exit after the $Generation update."
    $running = @(Get-SandboxProcesses $sandbox.App)
    if ($running.Count -ne 1 -or
        -not [string]::Equals($running[0].ExecutablePath, $sandbox.Target, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "The native app is not running after the $Generation update."
    }
    foreach ($argument in @('--skip-startup-update', '--update-complete')) {
        if ($running[0].CommandLine -notlike "*$argument*") {
            throw "The native app was started without $argument after the $Generation update."
        }
    }
    Stop-SandboxProcesses $sandbox.App
}

function Test-Rollback {
    param([string]$Generation, [string]$LegacyHost, [string]$FailingBridge)
    $sandbox = New-Sandbox "rollback-$Generation" $LegacyHost
    $updateDirectory = Invoke-LegacyUpdate $Generation $sandbox $FailingBridge
    $log = Get-UpdateLog $updateDirectory
    foreach ($expected in @('Update failed: The updated app exited during startup.', 'Previous executable restored and restarted.')) {
        if ($log -notlike "*$expected*") { throw "The $Generation update log is missing '$expected':`n$log" }
    }
    if ((Get-Sha256 $sandbox.Target) -ne (Get-Sha256 $LegacyHost)) { throw "The $Generation app was not restored." }
    $marker = $sandbox.Target + '.restored'
    $expectedMarker = ('restarted ' + $restartArguments[$Generation]).Trim()
    if (-not (Test-Path -LiteralPath $marker) -or "$(Get-Content -LiteralPath $marker -Raw)".Trim() -ne $expectedMarker) {
        throw "The $Generation app was not restarted after the rollback."
    }
    Stop-SandboxProcesses $sandbox.App
}

function Test-ManualStart {
    $sandbox = New-Sandbox 'manual' $Bridge
    $process = Start-Sandboxed $sandbox @('--skip-startup-update')
    if (-not $process.WaitForExit(60000)) { throw 'The downloaded file did not finish installing the native app.' }
    if ($process.ExitCode -ne 0) { throw "The downloaded file failed with exit code $($process.ExitCode)." }
    if ((Get-Sha256 $sandbox.Target) -ne (Get-Sha256 $NativeExecutable)) {
        throw 'Running the downloaded file did not install the native app.'
    }
    Stop-SandboxProcesses $sandbox.App
}

try {
    New-Item -ItemType Directory -Path $work | Out-Null
    $legacyHosts = [ordered]@{}
    foreach ($generation in $restartArguments.Keys) {
        $artifacts = Join-Path $work "legacy-host-$generation"
        Invoke-Build $hostProject $artifacts @("-p:LegacyVersion=$generation")
        $legacyHosts[$generation] = Join-Path $artifacts "bin\LegacyHost\release\$appName"
    }
    $failingArtifacts = Join-Path $work 'failing-bridge'
    Invoke-Build $bridgeProject $failingArtifacts @("-p:BridgeVersion=$Version", "-p:NativeExecutable=$($legacyHosts['0.1.7'])")
    $failingBridge = Join-Path $failingArtifacts "bin\BluetoothBatteryMonitor\release\$appName"

    foreach ($generation in $legacyHosts.Keys) {
        Test-AssetSelection $generation $legacyHosts[$generation]
        Write-Host "[$generation] asset selection: passed"
        Test-Migration $generation $legacyHosts[$generation]
        Write-Host "[$generation] update to the native app: passed"
        Test-Rollback $generation $legacyHosts[$generation] $failingBridge
        Write-Host "[$generation] rollback after a failed start: passed"
    }
    Test-ManualStart
    Write-Host 'Manual start of the downloaded file: passed'
} finally {
    Stop-SandboxProcesses $work
    Start-Sleep -Milliseconds 500
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
