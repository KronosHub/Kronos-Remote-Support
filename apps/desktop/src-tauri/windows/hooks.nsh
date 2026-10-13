; Installer hooks (bundle.windows.nsis.installerHooks).
;
; The stock "is the app running?" check uses the Restart Manager and aborts with
; "Failed to kill Kronos Remote Support" when a process using the executable cannot be
; shut down that way (seen on a PC with WithSecure). Before that check, close our own
; processes by name and wait for them to be gone, so it finds nothing left to stop.

!macro KRS_STOP_APP
  nsExec::Exec 'taskkill /F /T /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  ; Wait up to 10 s for the processes to really exit (an exiting process cannot be killed
  ; again, it just has to finish).
  StrCpy $1 0
  krs_wait:
    nsis_tauri_utils::FindProcess "${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 0
    ${AndIf} $1 < 20
      Sleep 500
      IntOp $1 $1 + 1
      Goto krs_wait
    ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro KRS_STOP_APP
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro KRS_STOP_APP
!macroend
