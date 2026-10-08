; Hush installer hooks (included by Tauri's NSIS template).
; VB-CABLE provides the virtual microphone. It is NOT bundled: its terms do not
; allow embedding it in another installer, so users install it themselves and
; Hush guides them. While Hush is installed, the VB-CABLE recording endpoint is
; shown as "Hush Microphone"; uninstalling gives it its own name back.

!define HUSH_VBCABLE_KEY "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VB:VBCABLE {87459874-1236-4469}"

!macro NSIS_HOOK_POSTINSTALL
  SetRegView 64
  ReadRegStr $0 HKLM "${HUSH_VBCABLE_KEY}" "DisplayName"
  SetRegView lastused
  ${If} $0 == ""
    MessageBox MB_OK|MB_ICONINFORMATION "Hush behöver också den kostnadsfria drivrutinen VB-CABLE från VB-Audio (vb-cable.com).$\r$\n$\r$\nNär Hush startar visas en knapp som tar dig till nedladdningen och en kort instruktion." /SD IDOK
  ${Else}
    nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --name-virtual-mic'
    Pop $1
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --restore-virtual-mic'
  Pop $1
!macroend
