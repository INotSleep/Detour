; The install folder on the user's PATH, and off it again.
;
; PATH is never read into NSIS: its strings end at 1024 characters, and a longer PATH
; read there comes back empty, which once replaced someone's whole PATH with this folder.
; PowerShell changes it in the registry instead: read as it is (%VARIABLES% unexpanded),
; written back as REG_EXPAND_SZ, and left alone if it can't be read. The folder reaches
; PowerShell in an environment variable, so no path needs quoting.
;
; `KEY` is under HKCU: "Environment", or a test key (see path-test.nsi). In NSIS strings
; PowerShell's `$` is written `$$`.

!macro DETOUR_PATH_RUN KEY SCRIPT
  System::Call 'Kernel32::SetEnvironmentVariable(t "DETOUR_DIR", t "$INSTDIR")i'
  System::Call 'Kernel32::SetEnvironmentVariable(t "DETOUR_KEY", t "${KEY}")i'
  nsExec::Exec `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "${SCRIPT}"`
  Pop $0
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=500
!macroend

; Adds the folder at the end, once; a ";" the PATH ended with isn't doubled.
!macro DETOUR_PATH_ADD KEY
  !insertmacro DETOUR_PATH_RUN "${KEY}" "$$ErrorActionPreference='Stop'; $$d=$$env:DETOUR_DIR; $$k=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($$env:DETOUR_KEY); $$p=[string]$$k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); if (@($$p -split ';' | Where-Object { $$_.TrimEnd('\') -eq $$d.TrimEnd('\') }).Count -eq 0) { $$t=$$p.TrimEnd(';'); $$k.SetValue('Path', $$(if ($$t) { $$t + ';' + $$d } else { $$d }), [Microsoft.Win32.RegistryValueKind]::ExpandString) }"
!macroend

; Takes the folder out; the rest stays as it was.
!macro DETOUR_PATH_REMOVE KEY
  !insertmacro DETOUR_PATH_RUN "${KEY}" "$$ErrorActionPreference='Stop'; $$d=$$env:DETOUR_DIR; $$k=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($$env:DETOUR_KEY, $$true); if ($$k) { $$p=[string]$$k.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $$all=$$p -split ';'; $$rest=@($$all | Where-Object { $$_.TrimEnd('\') -ne $$d.TrimEnd('\') }); if ($$rest.Count -ne $$all.Count) { $$k.SetValue('Path', ($$rest -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString) } }"
!macroend
