; Vesper Oriel's hooks into Tauri's NSIS installer (tauri.conf.json,
; bundle.windows.nsis.installerHooks).
;
; The installer installs per machine (Program Files, for every Windows account). Test builds
; before the first release installed per user (%LOCALAPPDATA%\Vesper Oriel, the account's own
; Start menu). Such an install is found under the installing account's HKCU and removed
; silently before the new files are copied, so the account doesn't keep a second, older Oriel
; in its Start menu. Its settings live in the account's own app-data folder and are kept (the
; uninstaller removes them only when its "Delete the application data" box is ticked, which a
; silent run never does). A per-user install of another account (when elevating with a
; different administrator's password) isn't seen: that account removes it in Settings > Apps.

!macro NSIS_HOOK_PREINSTALL
  Push $R8
  Push $R9
  ReadRegStr $R8 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}" "InstallLocation"
  ; Written quoted ("C:\...\Vesper Oriel").
  StrCpy $R9 $R8 1
  ${If} $R9 == '"'
    StrCpy $R8 $R8 -1 1
  ${EndIf}
  ${If} $R8 != ""
  ${AndIf} $R8 != $INSTDIR
  ${AndIf} ${FileExists} "$R8\uninstall.exe"
    DetailPrint "Removing the earlier per-user install in $R8 (its settings are kept)"
    ; _?= runs the uninstaller in place, so ExecWait waits for it.
    ExecWait '"$R8\uninstall.exe" /S _?=$R8' $R9
    DetailPrint "The earlier install's uninstaller ended with $R9"
    Delete "$R8\uninstall.exe"
    RMDir "$R8"
  ${EndIf}
  Pop $R9
  Pop $R8
!macroend
