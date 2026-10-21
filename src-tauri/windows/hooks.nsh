!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EZ Hanseikai" "InstallLocation"
  ${If} $0 != ""
    StrCpy $0 $0 -1 1
    !insertmacro CheckIfAppIsRunning "$0\ez-hanseikai.exe" "EZ Hanseikai"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EZ Hanseikai" "InstallLocation"
  ${If} $0 != ""
    StrCpy $0 $0 -1 1
    ExecWait '"$0\uninstall.exe" /S _?=$0'
    Delete "$0\uninstall.exe"
    RMDir "$0"
    DeleteRegKey HKCU "Software\ez-hanseikai\EZ Hanseikai"
    DeleteRegKey /ifempty HKCU "Software\ez-hanseikai"
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
    Rename "$APPDATA\dev.ez-hanseikai.app" "$APPDATA\dev.ggi-hanseikai.app"
  ${EndIf}
!macroend
