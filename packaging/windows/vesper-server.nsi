; Vesper server for Windows: installer (NSIS, cross-built on Linux by build-server.sh).
;
; Installs vesper.exe, FFmpeg's LGPL DLLs, the browser control UI and the licence texts into
; Program Files\Vesper, creates the data folder %ProgramData%\Vesper, allows vesper.exe through
; the Windows firewall and registers the scheduled task "Vesper" that runs the server headless
; at boot as SYSTEM (vesper-task.ps1), or, with "Show screens on this PC's monitors" (the
; "Screens" page; remembered for upgrades; `/DESKTOP` on the command line for a silent install),
; at the user's logon in their session, so display outputs can show on this PC's monitors.
; The account it then runs for is on the same page (`/USER=<name>`); an upgrade keeps the
; existing task's account unless it is changed there.
; Uninstalling keeps the data folder.
;
; makensis -DVERSION=0.1.0 -DSTAGE=<folder with the files> -DOUTFILE=<installer.exe> vesper-server.nsi

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
!include "nsDialogs.nsh"
!include "FileFunc.nsh"
!include "TextFunc.nsh"

Var OnDesktop
Var ScreensBox
Var DesktopUser
Var UserBox

Name "Vesper Server"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\Vesper"
; An earlier install's folder is read in .onInit (not with InstallDirRegKey, which this 32-bit
; installer reads in the 32-bit registry view, while the install writes the 64-bit one).
BrandingText "Vesper Server ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Vesper Server"
VIAddVersionKey "FileDescription" "Vesper Server installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "MIT OR Apache-2.0; FFmpeg: LGPL"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\VesperServer"

!define MUI_ICON "${STAGE}\vesper.ico"
!define MUI_UNICON "${STAGE}\vesper.ico"
!define MUI_FINISHPAGE_TEXT "The Vesper server now runs in the background (scheduled task $\"Vesper$\"): it starts with Windows, or, with screens on this PC's monitors, when you sign in.$\r$\n$\r$\nData: %ProgramData%\Vesper$\r$\nLog: %ProgramData%\Vesper\logs\vesper.log$\r$\n$\r$\nConnect Vesper Oriel to this computer, or open http://localhost:5555/ in a browser."
!insertmacro MUI_PAGE_LICENSE "${STAGE}\licenses\LICENSES.txt"
!insertmacro MUI_PAGE_DIRECTORY
Page custom ScreensPage ScreensLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; The 64-bit PowerShell (Sysnative: this 32-bit installer would otherwise get SysWOW64's).
!define POWERSHELL "$WINDIR\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
!macro PowerShell SCRIPT_ARGS
  nsExec::ExecToLog '"${POWERSHELL}" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$INSTDIR\vesper-task.ps1" ${SCRIPT_ARGS}'
  Pop $0
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Vesper needs 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext all
  ; An upgrade installs where the previous version is, unless /D= chose another folder.
  ${If} $INSTDIR == "$PROGRAMFILES64\Vesper"
    ReadRegStr $R2 HKLM "Software\Vesper\Server" "InstallDir"
    ${If} $R2 != ""
      StrCpy $INSTDIR $R2
    ${EndIf}
  ${EndIf}
  ; Headless unless chosen now (/DESKTOP) or at the previous install.
  ReadRegDWORD $OnDesktop HKLM "Software\Vesper\Server" "Desktop"
  ${If} $OnDesktop != 1
    StrCpy $OnDesktop 0
  ${EndIf}
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/DESKTOP" $R1
  ${IfNot} ${Errors}
    StrCpy $OnDesktop 1
  ${EndIf}
  ; The account the logon task runs for: /USER=, else the existing logon task's (an upgrade
  ; keeps it), else empty (the task script then takes the user signed in at the console).
  ClearErrors
  ${GetOptions} $R0 "/USER=" $R1
  ${IfNot} ${Errors}
    StrCpy $DesktopUser $R1
  ${Else}
    nsExec::ExecToStack '"${POWERSHELL}" -NoProfile -NonInteractive -Command "$$t = Get-ScheduledTask -TaskName Vesper -ErrorAction SilentlyContinue; if ($$t -and $$t.Principal.LogonType -eq $\'Interactive$\') { $$t.Principal.UserId }"'
    Pop $0
    Pop $1
    ${If} $0 == 0
      ${TrimNewLines} $1 $DesktopUser
    ${EndIf}
  ${EndIf}
FunctionEnd

; Where the server runs: headless at startup (a box whose screens go out over the network), or
; in the signed-in user's session so it can show screens on this PC's monitors.
Function ScreensPage
  !insertmacro MUI_HEADER_TEXT "Screens" "Where Vesper shows its screens"
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 48u "Vesper sends its screens over the network (IDN Open Protocol, compatible with NDI®). It can also show them full screen on monitors or projectors attached to this PC. For that it must run in your Windows session: it then starts when you sign in, not when Windows starts, and runs only while you are signed in."
  Pop $0
  ${NSD_CreateCheckbox} 0 56u 100% 12u "Show screens on this PC's monitors"
  Pop $ScreensBox
  ${If} $OnDesktop == 1
    ${NSD_Check} $ScreensBox
  ${EndIf}
  ${NSD_CreateLabel} 12u 72u 95% 24u "Leave this off for a server with no monitors (it then runs in the background from startup, before anyone signs in). You can change it by running the installer again."
  Pop $0
  ${NSD_CreateLabel} 12u 100u 95% 20u "Windows account it runs for (it starts when this account signs in; leave empty for the person signed in at this PC now):"
  Pop $0
  ${NSD_CreateText} 12u 122u 60% 12u "$DesktopUser"
  Pop $UserBox
  nsDialogs::Show
FunctionEnd

Function ScreensLeave
  ${NSD_GetText} $UserBox $DesktopUser
  ${NSD_GetState} $ScreensBox $OnDesktop
  ${If} $OnDesktop != ${BST_CHECKED}
    StrCpy $OnDesktop 0
  ${Else}
    StrCpy $OnDesktop 1
  ${EndIf}
FunctionEnd

; The installation failed after -Stop disabled the task (a file that couldn't be written and
; the operator gave up): the previous server's task is enabled and started again. Inline, not
; through vesper-task.ps1 -Resume (E11-1): the usual failure is vesper.exe or a DLL, before the
; new script is written, and the installed script may be one from before -Resume existed (it
; then stops at parameter binding and the task stays disabled). Enable and start the task by
; name works whatever script, if any, is installed; no task (a first install): nothing to do.
Function .onInstFailed
  nsExec::ExecToLog '"${POWERSHELL}" -NoProfile -NonInteractive -Command "if (Get-ScheduledTask -TaskName Vesper -ErrorAction SilentlyContinue) { Enable-ScheduledTask -TaskName Vesper -ErrorAction SilentlyContinue | Out-Null; Start-ScheduledTask -TaskName Vesper -ErrorAction SilentlyContinue }"'
  Pop $0
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Section "Vesper Server" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  ; An upgrade: stop the running server first, it holds vesper.exe and the DLLs.
  ${If} ${FileExists} "$INSTDIR\vesper-task.ps1"
    !insertmacro PowerShell "-Stop"
  ${EndIf}

  File "${STAGE}\vesper.exe"
  File "${STAGE}\*.dll"
  ; FFmpeg's tools came with batch 16's media preparation; IDN converts in the server now.
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
  File "${STAGE}\vesper.ico"
  File "${STAGE}\vesper-task.ps1"
  SetOutPath "$INSTDIR\ui"
  File /r "${STAGE}\ui\*"
  SetOutPath "$INSTDIR\scripture"
  File /r "${STAGE}\scripture\*"
  SetOutPath "$INSTDIR\licenses"
  File /r "${STAGE}\licenses\*"
  SetOutPath "$INSTDIR"

  ; Data folder (kept on uninstall).
  ; (The server only takes a folder with portable\ in it as its data.)
  CreateDirectory "$APPDATA\Vesper"
  CreateDirectory "$APPDATA\Vesper\portable"
  CreateDirectory "$APPDATA\Vesper\logs"

  ; The server as one of the Vesper Launcher's products (its updates, Launch and Stop), read
  ; by the Launcher when it is installed (its own installer). The Launcher's state folder is
  ; SYSTEM- and Administrators-only from here on (not ProgramData's inherited rights, which let
  ; Users create files); its owners are left as they are, so the Launcher's installer still
  ; finds files another user put there before (launcher-task.ps1 -Secure). vesper.json is
  ; written afresh (an administrator's file, not one already there).
  CreateDirectory "$APPDATA\Vesper Launcher\launcher.d"
  nsExec::ExecToLog 'icacls.exe "$APPDATA\Vesper Launcher" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F /T /C /Q'
  Pop $0
  Delete "$APPDATA\Vesper Launcher\launcher.d\vesper.json"
  SetOutPath "$APPDATA\Vesper Launcher\launcher.d"
  File "${STAGE}\launcher.d\vesper.json"
  SetOutPath "$INSTDIR"

  ; Firewall: the control API (5555), the site (5959, 29202), mDNS and IDN outputs all belong
  ; to vesper.exe, so one program rule covers them.
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Vesper Server"'
  Pop $0
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Vesper Server" dir=in action=allow program="$INSTDIR\vesper.exe" enable=yes profile=any'
  Pop $0

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "Software\Vesper\Server" "InstallDir" "$INSTDIR"
  WriteRegDWORD HKLM "Software\Vesper\Server" "Desktop" $OnDesktop
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "Vesper Server"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "Vesper"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\vesper.ico"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  ; Register the scheduled task and start the server now.
  DetailPrint "Registering the Vesper scheduled task"
  ${If} $OnDesktop == 1
  ${AndIf} $DesktopUser != ""
    !insertmacro PowerShell '-Install -Desktop -User "$DesktopUser"'
  ${ElseIf} $OnDesktop == 1
    ; The account box left empty: the person signed in at this PC now, as its label says (not
    ; the account an existing task runs for, which the box showed and was cleared).
    !insertmacro PowerShell "-Install -Desktop -ConsoleUser"
  ${Else}
    !insertmacro PowerShell "-Install"
  ${EndIf}
  ${If} $0 != 0
    MessageBox MB_ICONEXCLAMATION "The scheduled task could not be registered (code $0); a task registered before is kept as it was. See the details (for example a mistyped account name), or run vesper-task.ps1 -Install from an administrator PowerShell in $INSTDIR."
  ${EndIf}
SectionEnd

Section "Uninstall"
  ${If} ${FileExists} "$INSTDIR\vesper-task.ps1"
    !insertmacro PowerShell "-Remove"
  ${EndIf}
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="Vesper Server"'
  Pop $0

  Delete "$INSTDIR\vesper.exe"
  Delete "$INSTDIR\*.dll"
  Delete "$INSTDIR\ffmpeg.exe"
  Delete "$INSTDIR\ffprobe.exe"
  Delete "$INSTDIR\vesper.ico"
  Delete "$INSTDIR\vesper-task.ps1"
  RMDir /r "$INSTDIR\ui"
  RMDir /r "$INSTDIR\scripture"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$APPDATA\Vesper Launcher\launcher.d\vesper.json"

  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\Vesper\Server"
  DeleteRegKey /ifempty HKLM "Software\Vesper"
  ; %ProgramData%\Vesper (the church's libraries, media and settings) is kept.
SectionEnd
