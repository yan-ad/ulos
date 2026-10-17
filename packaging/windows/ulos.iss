; Build only through scripts/release-package.py; payloads are header-checked there.
#if VER != EncodeVer(6, 7, 3)
  #error Ulos requires the reviewed Inno Setup 6.7.3 compiler
#endif
#if PayloadArch == "arm64"
  #define InstallArchitecture "arm64"
#elif PayloadArch == "x86_64"
  ; Native x64 only: use the native ARM64 package on ARM PCs, not emulation.
  #define InstallArchitecture "x64os"
#else
  #error Unsupported Ulos payload architecture
#endif

[Setup]
AppId={{A723763F-953D-48CC-AF6C-0E235ED0DB19}
AppName=Ulos
AppVersion={#AppVersion}
AppVerName=Ulos {#AppVersion}
AppPublisher=Ulos contributors
AppPublisherURL=https://github.com/yan-ad/ulos
DefaultDirName={localappdata}\Programs\Ulos
PrivilegesRequired=lowest
ArchitecturesAllowed={#InstallArchitecture}
ArchitecturesInstallIn64BitMode={#InstallArchitecture}
LicenseFile={#StageDir}\LICENSE
InfoBeforeFile={#StageDir}\OSS-NOTICES.txt
OutputDir={#BuildOutputDir}
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
DisableWelcomePage=no
UninstallDisplayIcon={app}\Ulos.exe
CloseApplications=no
RestartApplications=no
RestartIfNeededByRun=no
ChangesAssociations=no
ChangesEnvironment=no

[Messages]
WizardLicense=Ulos MIT License
WizardInfoBefore=Third-party software notices
InfoBeforeLabel=Please read the third-party software notices. Complete notices and license texts are also installed with Ulos.
InfoBeforeClickLabel=When you are ready to continue, click Next.

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\Ulos"; Filename: "{app}\Ulos.exe"
Name: "{userprograms}\Ulos licenses and notices"; Filename: "{app}\OSS-NOTICES.txt"

; No Run/UninstallRun, association registry entries, service, or forced app close.
; Uninstall removes only installer-owned files; downloads/history are not purged.
