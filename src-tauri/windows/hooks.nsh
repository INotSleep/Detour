; Installer hooks (bundle.windows.nsis.installerHooks): the command line `detour` next to
; the app, and its folder on the user's PATH.
;
; Detour 0.2.3 and older had the window itself as detour.exe. Updating from it, Tauri's
; installer removes "the old main binary" after copying the new files, which is the
; command line by now: it is copied again after that. A running `detour mcp` (an AI
; client keeps it open) holds detour.exe; a running program can be renamed, so it is
; moved aside and keeps running, and the new one goes in its place.

!include WinMessages.nsh

${UnStrLoc}
${UnStrRep}

; The sidecar as Tauri gets it, next to this file's folder (see scripts/sidecar.mjs).
!define DETOUR_CLI "${__FILEDIR__}\..\binaries\detour-x86_64-pc-windows-msvc.exe"

!macro NSIS_HOOK_PREINSTALL
  Delete "$INSTDIR\detour.exe.old"
  Rename "$INSTDIR\detour.exe" "$INSTDIR\detour.exe.old"
  Delete /REBOOTOK "$INSTDIR\detour.exe.old"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File "/oname=detour.exe" "${DETOUR_CLI}"

  ; The install folder on the user's PATH, once.
  ReadRegStr $0 HKCU "Environment" "Path"
  ${StrLoc} $1 ";$0;" ";$INSTDIR;" ">"
  ${If} $1 == ""
    ${If} $0 == ""
      WriteRegExpandStr HKCU "Environment" "Path" "$INSTDIR"
    ${Else}
      WriteRegExpandStr HKCU "Environment" "Path" "$0;$INSTDIR"
    ${EndIf}
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=500
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; An update uninstalls the old version first: PATH stays as it is then.
  ${If} $UpdateMode <> 1
    Delete "$INSTDIR\detour.exe"
    Delete "$INSTDIR\detour.exe.old"
    ReadRegStr $0 HKCU "Environment" "Path"
    ${UnStrLoc} $1 ";$0;" ";$INSTDIR;" ">"
    ${If} $1 != ""
      ${UnStrRep} $1 ";$0;" ";$INSTDIR;" ";"
      ; Without the ";" added at both ends.
      StrCpy $1 $1 "" 1
      StrCpy $1 $1 -1
      WriteRegExpandStr HKCU "Environment" "Path" "$1"
      SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=500
    ${EndIf}
  ${EndIf}
!macroend
