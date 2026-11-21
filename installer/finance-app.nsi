; Установщик FinanceApp (NSIS). Сборка из корня репозитория:
;   makensis installer\finance-app.nsi
; Предварительно: cargo build -p finance-app --release --locked
Unicode true
!include "MUI2.nsh"

Name "FinanceApp"
OutFile "FinanceApp-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\FinanceApp"
RequestExecutionLevel user

!define MUI_ICON "..\assets\icons\icon.ico"
!define MUI_UNICON "..\assets\icons\icon.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Russian"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\target\release\finance-app.exe"
  File "..\assets\icons\icon.ico"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$SMPROGRAMS\FinanceApp"
  CreateShortcut "$SMPROGRAMS\FinanceApp\FinanceApp.lnk" "$INSTDIR\finance-app.exe" "" "$INSTDIR\icon.ico"
  CreateShortcut "$DESKTOP\FinanceApp.lnk" "$INSTDIR\finance-app.exe" "" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FinanceApp" "DisplayName" "FinanceApp"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FinanceApp" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FinanceApp" "UninstallString" "$INSTDIR\uninstall.exe"
SectionEnd

; Данные пользователя в %APPDATA%\FinanceApp намеренно не удаляются.
Section "Uninstall"
  Delete "$INSTDIR\finance-app.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\FinanceApp\FinanceApp.lnk"
  RMDir "$SMPROGRAMS\FinanceApp"
  Delete "$DESKTOP\FinanceApp.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\FinanceApp"
SectionEnd
