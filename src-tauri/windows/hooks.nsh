; Installer hooks (bundle.windows.nsis.installerHooks): the command line `detour` next to
; the app, and its folder on the user's PATH.
;
; Detour 0.2.3 and older had the window itself as detour.exe. Updating from it, Tauri's
; installer removes "the old main binary" after copying the new files, which is the
; command line by now: it is copied again after that. A running `detour mcp` (an AI
; client keeps it open) holds detour.exe; a running program can be renamed, so it is
; moved aside and keeps running, and the new one goes in its place.

!include WinMessages.nsh
!include "${__FILEDIR__}\path.nsh"

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
  !insertmacro DETOUR_PATH_ADD "Environment"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; An update uninstalls the old version first: PATH stays as it is then.
  ${If} $UpdateMode <> 1
    Delete "$INSTDIR\detour.exe"
    Delete "$INSTDIR\detour.exe.old"
    !insertmacro DETOUR_PATH_REMOVE "Environment"
  ${EndIf}
!macroend
