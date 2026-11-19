; Inno Setup 6 script for CCANH (ADR 0010). Build with scripts/build-installer.ps1,
; which passes AppVersion and the path to the release binary.
;
; Installs crane.exe to Program Files. Data (database, logs, keys, config.toml)
; lives in %ProgramData%\CraneCamera, which uninstalling keeps.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\crane.exe"
#endif

[Setup]
AppId={{2C742141-CDE9-4529-9DBE-7B984F1AB2F6}
AppName=Crane Camera Automated Navigation Hub
AppVersion={#AppVersion}
AppPublisher=CCANH
DefaultDirName={autopf}\CCANH
DefaultGroupName=CCANH
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=crane-setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\crane.exe
SetupIconFile=crane.ico

[Tasks]
Name: "service"; Description: "Run as a Windows service that starts with Windows (recommended)"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "crane.exe"; Flags: ignoreversion
Source: "..\config.example.toml"; DestDir: "{app}"; Flags: ignoreversion

[Dirs]
Name: "{commonappdata}\CraneCamera"

[Icons]
Name: "{group}\Open CCANH"; Filename: "http://127.0.0.1:8080/"; IconFilename: "{app}\crane.exe"
Name: "{group}\Data folder"; Filename: "{commonappdata}\CraneCamera"
Name: "{group}\Run CCANH in a console"; Filename: "{app}\crane.exe"; Parameters: "--data-dir ""{commonappdata}\CraneCamera"""; Tasks: not service
Name: "{group}\Uninstall CCANH"; Filename: "{uninstallexe}"

[Run]
; The data folder holds the database (password hashes, camera credentials
; encrypted with the machine DPAPI key): only SYSTEM and Administrators may
; read it. ProgramData would otherwise let every local user read it.
Filename: "{sys}\icacls.exe"; Parameters: """{commonappdata}\CraneCamera"" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F"; Flags: runhidden waituntilterminated; StatusMsg: "Restricting access to the data folder..."
; Upgrades: remove an existing service first (ignore failure if there is none).
Filename: "{app}\crane.exe"; Parameters: "--uninstall-service"; Flags: runhidden waituntilterminated; Tasks: service; StatusMsg: "Removing the previous service..."
Filename: "{app}\crane.exe"; Parameters: "--install-service --data-dir ""{commonappdata}\CraneCamera"""; Flags: runhidden waituntilterminated; Tasks: service; StatusMsg: "Registering the service..."
; Restart automatically if the process ever crashes (5 s, 10 s, then 60 s; counter resets daily).
Filename: "{sys}\sc.exe"; Parameters: "failure CraneCameraHub reset= 86400 actions= restart/5000/restart/10000/restart/60000"; Flags: runhidden waituntilterminated; Tasks: service
Filename: "{sys}\sc.exe"; Parameters: "start CraneCameraHub"; Flags: runhidden waituntilterminated; Tasks: service; StatusMsg: "Starting the service..."
Filename: "http://127.0.0.1:8080/"; Description: "Open CCANH in the browser (first run: use the setup token from the log in the data folder)"; Flags: postinstall shellexec skipifsilent nowait unchecked

[UninstallRun]
Filename: "{app}\crane.exe"; Parameters: "--uninstall-service"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"
