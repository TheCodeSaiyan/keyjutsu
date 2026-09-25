; KeyJutsu's additions to the Tauri NSIS installer (Milestone 16).
;
; After installing, two questions: put `keyjutsu` on the PATH, and add
; "Open KeyJutsu here" to Explorer's folder menus. A silent install (/S)
; answers yes to both. Each is done by the installed CLI itself
; (`keyjutsu setup path|explorer add`), so the installer and the product
; agree on what was changed, and the uninstaller takes both out again
; before the files go.

!macro NSIS_HOOK_POSTINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION "Add the keyjutsu command to your PATH, so terminals can run it?" /SD IDYES IDNO kj_skip_path
    nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup path add'
  kj_skip_path:
  MessageBox MB_YESNO|MB_ICONQUESTION "Add $\"Open KeyJutsu here$\" to Explorer's folder menus?" /SD IDYES IDNO kj_skip_explorer
    nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup explorer add'
  kj_skip_explorer:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup path remove'
  nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup explorer remove'
!macroend
