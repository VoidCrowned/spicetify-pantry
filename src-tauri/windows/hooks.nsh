; Spicetify Pantry used to be called "Pantry" (up to 0.3.x). A new product name installs as a separate app,
; so remove the old entry silently first. Only if it's really ours: same publisher and binary.
; Profiles live in %APPDATA%\Spicetify Pantry (moved from %APPDATA%\Pantry on first start) and aren't touched by either installer.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pantry" "Publisher"
  ReadRegStr $R1 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pantry" "MainBinaryName"
  ReadRegStr $R2 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pantry" "InstallLocation"
  ${If} $R0 == "github"
  ${AndIf} $R1 == "pantry.exe"
  ${AndIf} $R2 != ""
    DetailPrint "Removing the old Pantry install…"
    ; InstallLocation is stored with quotes.
    StrCpy $R3 $R2 1
    ${If} $R3 == '"'
      StrCpy $R2 $R2 -1 1
    ${EndIf}
    ExecWait '"$R2\uninstall.exe" /S _?=$R2'
    Delete "$R2\uninstall.exe"
    RMDir "$R2"
  ${EndIf}
!macroend
