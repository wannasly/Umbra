Unicode True
!include "MUI2.nsh"
!include "x64.nsh"
!ifndef UMBRA_VERSION
  !define UMBRA_VERSION "0.3.0"
!endif
Name "Umbra"
OutFile "${__FILEDIR__}\..\release\Umbra_${UMBRA_VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\Umbra"
InstallDirRegKey HKCU "Software\Umbra" "InstallDir"
RequestExecutionLevel user
!define MUI_ICON "${__FILEDIR__}\..\src-tauri\icons\icon.ico"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Russian"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Umbra requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

!macro CheckRunning PREFIX
Function ${PREFIX}CheckRunning
  loop:
    FindWindow $0 "" "Umbra"
    IntCmp $0 0 probe in_use in_use
  probe:
    ; A hidden native window may not be discoverable. Windows locks a running
    ; executable against writes, so probe the destination without changing it.
    IfFileExists "$INSTDIR\Umbra.exe" 0 done
    ClearErrors
    FileOpen $0 "$INSTDIR\Umbra.exe" a
    IfErrors in_use
    FileClose $0
    Goto done
  in_use:
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Exit Umbra using Quit in its tray menu, then click Retry." IDRETRY loop
    Abort
  done:
FunctionEnd
!macroend
!insertmacro CheckRunning ""
!insertmacro CheckRunning "un."

Section "Umbra"
  Call CheckRunning
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /oname=Umbra.exe "${__FILEDIR__}\..\target\release\umbra.exe"
  File /oname=LICENSE.txt "${__FILEDIR__}\..\LICENSE"
  File "${__FILEDIR__}\..\THIRD_PARTY_NOTICES.md"
  SetOutPath "$INSTDIR\licenses"
  File "${__FILEDIR__}\..\licenses\*"
  SetOutPath "$INSTDIR\resources"
  File "${__FILEDIR__}\..\src-tauri\resources\sing-box.exe"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Umbra.lnk" "$INSTDIR\Umbra.exe"
  WriteRegStr HKCU "Software\Umbra" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra" "DisplayName" "Umbra"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra" "DisplayVersion" "${UMBRA_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra" "Publisher" "Umbra"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra" "DisplayIcon" "$INSTDIR\Umbra.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
SectionEnd

Section "Uninstall"
  Call un.CheckRunning
  SetShellVarContext current
  ReadRegStr $0 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Umbra"
  StrCmp $0 '$\"$INSTDIR\Umbra.exe$\" --minimized' 0 +2
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Umbra"
  Delete "$SMPROGRAMS\Umbra.lnk"
  Delete "$INSTDIR\Umbra.exe"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\licenses\Slint-Royalty-free-2.0.md"
  Delete "$INSTDIR\licenses\Lucide-LICENSE.txt"
  Delete "$INSTDIR\licenses\flag-icons-LICENSE.txt"
  Delete "$INSTDIR\licenses\inter-OFL.txt"
  Delete "$INSTDIR\licenses\manrope-OFL.txt"
  Delete "$INSTDIR\licenses\jetbrainsmono-OFL.txt"
  Delete "$INSTDIR\licenses\sing-box-GPL-3.0.txt"
  Delete "$INSTDIR\licenses\GPL-3.0.txt"
  Delete "$INSTDIR\licenses\Rust-dependencies.txt"
  RMDir "$INSTDIR\licenses"
  Delete "$INSTDIR\resources\sing-box.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR\resources"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Umbra"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Umbra"
  ; User profiles and settings in APPDATA are intentionally preserved.
SectionEnd
