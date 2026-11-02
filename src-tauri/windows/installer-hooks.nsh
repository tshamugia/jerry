; Jerry's hooks into Tauri's NSIS installer template (bundle > windows > nsis > installerHooks).
;
; The template !includes this file before it defines PRODUCTNAME, BUNDLEID and the rest, so code
; that NSIS reads right here (defines, functions) spells Jerry's names out. The NSIS_HOOK_* macros
; expand later, inside the template's sections, and can use the template's defines and variables.

; Tauri installs a per-user app into %LOCALAPPDATA%\<productName>, but %LOCALAPPDATA%\Jerry holds
; Jerry's logs. Jerry goes into %LOCALAPPDATA%\Programs\Jerry instead, like other per-user apps.
!define JERRY_TEMPLATE_INSTDIR "$LOCALAPPDATA\Jerry"
!define JERRY_INSTDIR "$LOCALAPPDATA\Programs\Jerry"

; MUI calls this from .onGUIInit: after the template's .onInit has set its default folder (or
; restored a previous install's) and before the directory page shows it.
!define MUI_CUSTOMFUNCTION_GUIINIT JerryGuiInit
Function JerryGuiInit
  ${If} $INSTDIR == "${JERRY_TEMPLATE_INSTDIR}"
    StrCpy $INSTDIR "${JERRY_INSTDIR}"
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; A silent install (/S) has no GUI, so JerryGuiInit never ran. The template has already called
  ; SetOutPath, which created the default folder: move on and remove it again if it stayed empty.
  ${If} ${Silent}
  ${AndIf} $INSTDIR == "${JERRY_TEMPLATE_INSTDIR}"
    StrCpy $INSTDIR "${JERRY_INSTDIR}"
    SetOutPath $INSTDIR
    RMDir "${JERRY_TEMPLATE_INSTDIR}"
  ${EndIf}
!macroend

; Every registry entry Jerry or its installer writes goes away on uninstall (the registry list in
; the Tech PRD). The template already deletes the Uninstall key, the HKCU Run value (autostart) and
; the Start menu shortcut. A new registry entry in Jerry goes into this list and the Tech PRD's.
;
; An update (/UPDATE, from the updater) runs the old uninstaller too; it keeps all of this.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    ; Install location and installer language. The template deletes these only together with
    ; the app data.
    DeleteRegKey HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"

    ; Autostart: the enabled/disabled state Windows keeps next to the Run value.
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"

    ; "Open in Jerry" in Explorer's context menu.
    DeleteRegKey HKCU "Software\Classes\Directory\shell\${PRODUCTNAME}"
    DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\${PRODUCTNAME}"
    DeleteRegKey HKCU "Software\Classes\Drive\shell\${PRODUCTNAME}"

    ; Toast settings Windows keeps for Jerry's AUMID (the identifier on the Start menu shortcut).
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\${BUNDLEID}"

    ; "Delete the application data": the template deletes the WebView2 data in the BUNDLEID
    ; folders; Jerry's own data is jerry.db in %APPDATA%\Jerry and the logs in %LOCALAPPDATA%\Jerry.
    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "$APPDATA\${PRODUCTNAME}"
      RMDir /r "$LOCALAPPDATA\${PRODUCTNAME}"
    ${EndIf}
  ${EndIf}
!macroend
