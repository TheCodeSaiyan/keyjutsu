; KeyJutsu's additions to the Tauri NSIS installer.
;
; After installing, two questions: put `keyjutsu` on the PATH, and add
; "Open KeyJutsu here" to Explorer's folder menus. A silent install (/S)
; answers yes to both. Each is done by the installed CLI itself
; (`keyjutsu setup path|explorer add`), so the installer and the product
; agree on what was changed, and the uninstaller takes both out again
; before the files go.

; Never replace or remove KeyJutsu while a plan is changing the machine
; (ADR 0019). A run or recovery holds the machine-wide semaphore
; Global\KeyJutsu.run for as long as it lasts, and Windows removes it with
; the last handle, so it exists only while one is going. It is opened, not
; made: an installer must not take the lock itself. Access denied means it
; exists too, held by another account. A silent install exits 3 without
; changing anything. The uninstaller checks as well, because an upgrade can
; run the old uninstaller before this installer's own check.
!macro KJ_REFUSE_WHILE_RUNNING
  System::Call 'kernel32::OpenSemaphoreW(i 0x00100000, i 0, w "Global\KeyJutsu.run") p .r0 ?e'
  Pop $1
  StrCmp $0 0 kj_lock_absent
    System::Call 'kernel32::CloseHandle(p r0)'
    Goto kj_lock_held
  kj_lock_absent:
  StrCmp $1 5 kj_lock_held kj_lock_free
  kj_lock_held:
    MessageBox MB_OK|MB_ICONSTOP "KeyJutsu is running a plan on this computer. Let it finish, then try again. Nothing was changed." /SD IDOK
    SetErrorLevel 3
    Quit
  kj_lock_free:
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro KJ_REFUSE_WHILE_RUNNING
!macroend

!macro NSIS_HOOK_POSTINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION "Add the keyjutsu command to your PATH, so terminals can run it?" /SD IDYES IDNO kj_skip_path
    nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup path add'
  kj_skip_path:
  MessageBox MB_YESNO|MB_ICONQUESTION "Add $\"Open KeyJutsu here$\" to Explorer's folder menus?" /SD IDYES IDNO kj_skip_explorer
    nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup explorer add'
  kj_skip_explorer:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro KJ_REFUSE_WHILE_RUNNING
  nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup path remove'
  nsExec::ExecToLog '"$INSTDIR\keyjutsu.exe" setup explorer remove'
!macroend

; What the elevation broker captured before Administrator steps, kept in
; %ProgramData%\KeyJutsu where only Administrators can write (ADR 0017).
; It belongs to the installed program, not to anyone's history, so it goes
; with it. With the all-users context, $APPDATA is ProgramData.
!macro NSIS_HOOK_POSTUNINSTALL
  SetShellVarContext all
  RMDir /r "$APPDATA\KeyJutsu"
!macroend
