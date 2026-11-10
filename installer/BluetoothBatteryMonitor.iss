#define MyAppName "Bluetooth Battery Monitor"
#define MyAppPublisher "jeremygold02"
#define MyAppURL "https://github.com/jeremygold02/bluetooth-battery-monitor"
#define MyAppExe "BluetoothBatteryMonitor.exe"

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

[Setup]
AppId={{85851350-9814-45C4-8B2B-0E2E97A09F54}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases/latest
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExe}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=output
OutputBaseFilename=BluetoothBatteryMonitor-Setup
SetupIconFile=..\icon.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
Type: filesandordirs; Name: "{app}\licenses"

[Files]
Source: "..\target\release\BluetoothBatteryMonitor.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\artifacts\build\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\artifacts\build\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[UninstallDelete]
Type: files; Name: "{app}\.{#MyAppExe}.update*"

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[Code]
function GetCurrentProcessId: Cardinal;
  external 'GetCurrentProcessId@kernel32.dll stdcall';
function ProcessIdToSessionId(ProcessId: Cardinal; var SessionId: Cardinal): Boolean;
  external 'ProcessIdToSessionId@kernel32.dll stdcall';

{ Only signal the existing app when it is this exact installation in this session.
  The app's exit event is shared with portable copies, so never signal on a name alone. }
function InstalledProcessState: Integer;
var
  Locator, Services, Processes, Process: Variant;
  SessionId: Cardinal;
  Index: Integer;
  ProcessPath, InstalledPath: String;
begin
  Result := -1;
  if not ProcessIdToSessionId(GetCurrentProcessId, SessionId) then Exit;
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Services := Locator.ConnectServer('.', 'root\CIMV2');
    Processes := Services.ExecQuery('SELECT ExecutablePath, SessionId FROM Win32_Process WHERE Name = ''BluetoothBatteryMonitor.exe''');
    InstalledPath := ExpandConstant('{app}\{#MyAppExe}');
    Result := 0;
    for Index := 0 to Processes.Count - 1 do begin
      Process := Processes.ItemIndex(Index);
      if Cardinal(Process.SessionId) = SessionId then begin
        if VarIsNull(Process.ExecutablePath) then begin
          Result := -1;
          Exit;
        end;
        ProcessPath := Process.ExecutablePath;
        if CompareText(ProcessPath, InstalledPath) = 0 then begin
          Result := 1;
          Exit;
        end;
      end;
    end;
  except
    Log('Could not determine whether this installation is running: ' + GetExceptionMessage);
    Result := -1;
  end;
end;

function CloseInstalledApp: Boolean;
var
  State, ExitCode, Attempt: Integer;
  ExePath: String;
begin
  Result := False;
  ExePath := ExpandConstant('{app}\{#MyAppExe}');
  if not FileExists(ExePath) then begin
    Result := True;
    Exit;
  end;
  State := InstalledProcessState;
  if State = 0 then begin
    Result := True;
    Exit;
  end;
  if State <> 1 then Exit;
  if not Exec(ExePath, '--exit', '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then Exit;
  for Attempt := 1 to 40 do begin
    Sleep(250);
    State := InstalledProcessState;
    if State = 0 then begin
      Result := True;
      Exit;
    end;
    if State < 0 then Exit;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not CloseInstalledApp then
    Result := 'Could not safely close the installed Bluetooth Battery Monitor. Exit it from the tray, then try again.';
end;

function InitializeUninstall: Boolean;
begin
  Result := CloseInstalledApp;
  if not Result then
    MsgBox('Could not safely close the installed Bluetooth Battery Monitor. Exit it from the tray, then try again.', mbError, MB_OK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StartupValue, InstalledCommand: String;
begin
  if CurUninstallStep <> usUninstall then Exit;
  InstalledCommand := '"' + ExpandConstant('{app}\{#MyAppExe}') + '"';
  if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
    'BluetoothBatteryMonitor', StartupValue) and (CompareText(StartupValue, InstalledCommand) = 0) then
    RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'BluetoothBatteryMonitor');
end;
