; Vesper Launcher for Windows: installer (NSIS, cross-built on Linux by build-launcher.sh).
;
; Installs the Launcher service (vesper-launcher.exe, run by the scheduled task "Vesper Launcher"
; as SYSTEM from boot: launcher-task.ps1), its tray icon (vesper-launcher-tray.exe, started at
; every sign-in), and the bundled IDN Node and tools (idn-node\: idn-node, idn-session, idn,
; idn-display) into Program Files\Vesper Launcher. Its state folder is
; %ProgramData%\Vesper Launcher (launcher.json, the site's Launcher key, kept packages). On its
; own it makes a display box or a control PC; the Vesper server's installer adds the server as
; one of its products (launcher.d\vesper.json). Silent: `/S` (the Launcher's own self-update
; runs it so). Uninstalling keeps the state folder.
;
; makensis -DVERSION=0.1.0 -DSTAGE=<folder with the files> -DOUTFILE=<installer.exe> vesper-launcher.nsi

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel admin
ManifestDPIAware true

!ifndef VERSION
  !error "VERSION is not defined"
!endif
!ifndef STAGE
  !error "STAGE is not defined"
!endif
!ifndef OUTFILE
  !error "OUTFILE is not defined"
!endif

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"

Name "Vesper Launcher"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\Vesper Launcher"
BrandingText "Vesper Launcher ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Vesper Launcher"
VIAddVersionKey "FileDescription" "Vesper Launcher installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VesperLauncher"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define STATE "$APPDATA\Vesper Launcher"

!define MUI_ICON "${STAGE}\vesper-launcher.ico"
!define MUI_UNICON "${STAGE}\vesper-launcher.ico"
!define MUI_FINISHPAGE_TEXT "The Vesper Launcher now runs in the background (scheduled task $\"Vesper Launcher$\", from Windows' start). Its tray icon starts when you sign in: it opens the first-run setup (this PC's role, joining the site).$\r$\n$\r$\nIts window is also at http://127.0.0.1:5556/ in a browser on this PC."
!insertmacro MUI_PAGE_LICENSE "${STAGE}\licenses\LICENSES.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; The 64-bit PowerShell (Sysnative: this 32-bit installer would otherwise get SysWOW64's).
!define POWERSHELL "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
!macro Task SCRIPT_ARGS
  nsExec::ExecToLog '"${POWERSHELL}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\launcher-task.ps1" ${SCRIPT_ARGS}'
  Pop $0
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Vesper needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext all
  ${If} $INSTDIR == "$PROGRAMFILES64\Vesper Launcher"
    ReadRegStr $R2 HKLM "Software\Vesper\Launcher" "InstallDir"
    ${If} $R2 != ""
      StrCpy $INSTDIR $R2
    ${EndIf}
  ${EndIf}
FunctionEnd

; The installation failed after -Stop: the Launcher that was there runs again.
Function .onInstFailed
  nsExec::ExecToLog '"${POWERSHELL}" -NoProfile -NonInteractive -Command "if (Get-ScheduledTask -TaskName $\'Vesper Launcher$\' -ErrorAction SilentlyContinue) { Enable-ScheduledTask -TaskName $\'Vesper Launcher$\' -ErrorAction SilentlyContinue | Out-Null; Start-ScheduledTask -TaskName $\'Vesper Launcher$\' -ErrorAction SilentlyContinue }"'
  Pop $0
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Section "Vesper Launcher" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; An upgrade: the Launcher, the tray icons and the IDN programs hold their files. This
  ; version's script ends them (it notes the session agents and trays it ends, and -Install
  ; starts them again in their users' sessions).
  ${If} ${FileExists} "$INSTDIR\launcher-task.ps1"
    InitPluginsDir
    File "/oname=$PLUGINSDIR\launcher-task.ps1" "${STAGE}\launcher-task.ps1"
    nsExec::ExecToLog '"${POWERSHELL}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\launcher-task.ps1" -Stop -Dir "$INSTDIR"'
    Pop $0
  ${EndIf}

  File "${STAGE}\vesper-launcher.exe"
  File "${STAGE}\vesper-launcher-tray.exe"
  File "${STAGE}\WebView2Loader.dll"
  File "${STAGE}\vesper-launcher.ico"
  File "${STAGE}\launcher-task.ps1"
  SetOutPath "$INSTDIR\idn-node"
  File /r "${STAGE}\idn-node\*"
  SetOutPath "$INSTDIR\licenses"
  File /r "${STAGE}\licenses\*"
  SetOutPath "$INSTDIR"

  ; The configuration: the Launcher itself as a product (its self-update). Kept when an admin
  ; edited it; products of their own (the server's launcher.d\vesper.json) come from their
  ; installers. The state folder is made safe first: what another user put there (before an
  ; install, Users may create folders under ProgramData) is renamed `.untrusted`, never kept
  ; as the configuration the SYSTEM task loads.
  !insertmacro Task "-Secure"
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "The Launcher's state folder ${STATE} could not be made safe (code $0). See the details."
    Abort
  ${EndIf}
  CreateDirectory "${STATE}"
  CreateDirectory "${STATE}\launcher.d"
  SetOutPath "${STATE}"
  SetOverwrite off
  File "${STAGE}\launcher.json"
  SetOverwrite on
  SetOutPath "$INSTDIR"

  ; Firewall: the IDN programs receive and send video (the Launcher itself listens on
  ; 127.0.0.1 only).
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Vesper IDN Node"'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Vesper IDN Node" dir=in action=allow program="$INSTDIR\idn-node\idn-node.exe" enable=yes profile=any'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Vesper IDN Node" dir=in action=allow program="$INSTDIR\idn-node\idn.exe" enable=yes profile=any'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Vesper IDN Node" dir=in action=allow program="$INSTDIR\idn-node\idn-display.exe" enable=yes profile=any'
  Pop $0

  ; The tray icon at every sign-in, for every user.
  WriteRegStr HKLM "${RUN_KEY}" "Vesper Launcher" '"$INSTDIR\vesper-launcher-tray.exe"'

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Vesper\Launcher" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "Vesper Launcher"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Vesper"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\vesper-launcher.ico"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  DetailPrint "Registering the Vesper Launcher scheduled task"
  !insertmacro Task "-Install"
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "The scheduled task could not be registered (code $0). See the details, or run launcher-task.ps1 -Install from an administrator PowerShell in $INSTDIR."
  ${EndIf}
SectionEnd

Section "Uninstall"
  ; -Remove first removes the IDN Node's service when it runs this copy (the Launcher made this
  ; PC a display box; its state, the machine's IDN identity, stays in %ProgramData%\IDN Node).
  ${If} ${FileExists} "$INSTDIR\launcher-task.ps1"
    !insertmacro Task "-Remove"
  ${EndIf}
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Vesper IDN Node"'
  Pop $0
  DeleteRegValue HKLM "${RUN_KEY}" "Vesper Launcher"

  Delete "$INSTDIR\vesper-launcher.exe"
  Delete "$INSTDIR\vesper-launcher-tray.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\vesper-launcher.ico"
  Delete "$INSTDIR\launcher-task.ps1"
  RMDir /r "$INSTDIR\idn-node"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\Vesper\Launcher"
  DeleteRegKey /ifempty HKLM "Software\Vesper"
  ; %ProgramData%\Vesper Launcher (its configuration, the site's key, kept packages) is kept.
SectionEnd
