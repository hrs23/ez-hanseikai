!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GGI Hanseikai" "InstallLocation"
  ${If} $0 != ""
    StrCpy $0 $0 -1 1
    !insertmacro CheckIfAppIsRunning "$0\ggi-hanseikai.exe" "GGI Hanseikai"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\GGI Hanseikai" "InstallLocation"
  ${If} $0 != ""
    StrCpy $0 $0 -1 1
    ExecWait '"$0\uninstall.exe" /S _?=$0'
    Delete "$0\uninstall.exe"
    RMDir "$0"
    DeleteRegKey HKCU "Software\ggi-hanseikai\GGI Hanseikai"
    DeleteRegKey /ifempty HKCU "Software\ggi-hanseikai"
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${If} $UpdateMode = 1
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}
    Rename "$APPDATA\dev.ggi-hanseikai.app" "$APPDATA\${BUNDLEID}"
  ${EndIf}
!macroend
