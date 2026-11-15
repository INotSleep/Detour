; Tries path.nsh on a test key instead of the real PATH:
;   makensis /DACTION=ADD path-test.nsi       (and /DACTION=REMOVE)
;   path-test-add.exe, then HKCU\Software\DetourPathTest\Path shows what it did.
Unicode true
!include LogicLib.nsh
!include WinMessages.nsh
!include "${__FILEDIR__}\path.nsh"

Name "Detour PATH test"
OutFile "path-test-${ACTION}.exe"
RequestExecutionLevel user
SilentInstall silent
InstallDir "$LOCALAPPDATA\Detour"

Section
  !insertmacro DETOUR_PATH_${ACTION} "Software\DetourPathTest"
SectionEnd
