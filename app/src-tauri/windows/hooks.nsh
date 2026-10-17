; Hooks into Tauri's NSIS installer (bundle > windows > nsis > installerHooks).

; Installers up to 1.0.1-BETA put the export-bindings dev tool beside the app.
; An update (/UPDATE, as the in-app updater runs it) never runs the old
; uninstaller, and the new one only deletes the files it installs itself, so
; the old copy would stay for good.
!macro NSIS_HOOK_POSTINSTALL
  Delete "$INSTDIR\export-bindings.exe"
!macroend

; A copy the update couldn't delete. PRE, not POST: the uninstaller's
; RMDir "$INSTDIR" only removes an empty folder and runs before the POST hook.
!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\export-bindings.exe"
!macroend
