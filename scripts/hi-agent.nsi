; hi-agent.nsi -- Windows installer for Hi Agent (NSIS / Modern UI 2).
;
; The Windows analog of scripts/make-dmg.sh. Produces a per-user, no-admin
; Setup.exe that drops the (cross-compiled) hi-agent.exe under
; %LOCALAPPDATA%\Programs\Hi Agent, wires up Start Menu + Desktop shortcuts and
; an Add/Remove-Programs entry, and ships an uninstaller. The managed runtime
; (Node + codex + esbuild + ffmpeg + models) is NOT bundled here -- the binary
; auto-provisions it into the OS cache on first launch (the "it runs" tier).
;
; The shell (app/windows, WinUI 3) is carried only when SHELLDIR is defined,
; because it can only be built on a Windows host and this installer is built on
; the Mac mini. Without it the installer still produces a working install: the
; shortcuts point at hi-agent.exe and the person gets a headless core they open
; in a browser. With it they point at HiAgent.exe and the engine becomes the
; shell's child. One installer, two payload tiers, no second script.
;
; Driven entirely by /D defines from scripts/make-installer.sh:
;   VERSION   display version, e.g. 0.1.0       (default below)
;   VERSION4  four-part numeric, e.g. 0.1.0.0   (for VIProductVersion)
;   SRCEXE    path to the built hi-agent.exe
;   ICON      path to HiAgent.ico
;   SHELLDIR  optional: `dotnet publish` output of the WinUI shell
;   OUTFILE   output Setup.exe path

Unicode true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef VERSION4
  !define VERSION4 "0.0.0.0"
!endif
!ifndef SRCEXE
  !define SRCEXE "..\target\x86_64-pc-windows-msvc\release\hi-agent.exe"
!endif
!ifndef ICON
  !define ICON "HiAgent.ico"
!endif
!ifndef OUTFILE
  !define OUTFILE "..\target\installer\hi-agent-${VERSION}-windows-x64.exe"
!endif

!define APPNAME    "Hi Agent"
!define PUBLISHER  "Human Interface"
!define EXENAME    "hi-agent.exe"
!define SHELLEXE   "HiAgent.exe"

; What the shortcuts start. The shell when there is one; the engine alone
; otherwise, which is a core with no face on this machine but still a core.
!ifdef SHELLDIR
  !define LAUNCHER "${SHELLEXE}"
!else
  !define LAUNCHER "${EXENAME}"
!endif

; Program files go in a subdirectory of the chosen install location, never
; directly in it. The uninstaller can then remove that subdirectory whole --
; the shell publishes hundreds of files and listing them here would rot -- and
; a person who picked an existing folder on the directory page does not have it
; deleted out from under them.
!define APPDIR "$INSTDIR\app"
; Add/Remove-Programs key (per-user). Stable id, not the display name.
!define ARP_KEY    "Software\Microsoft\Windows\CurrentVersion\Uninstall\hi-agent"

Name "${APPNAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
; Per-user install -- no UAC elevation, like Chrome's default consumer install.
RequestExecutionLevel user
InstallDirRegKey HKCU "Software\hi-agent" "InstallDir"
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION4}"
VIAddVersionKey "ProductName"     "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "CompanyName"     "${PUBLISHER}"
VIAddVersionKey "ProductVersion"  "${VERSION}"
VIAddVersionKey "FileVersion"     "${VERSION4}"
VIAddVersionKey "LegalCopyright"  "(C) ${PUBLISHER}"

!define MUI_ICON   "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "${APPDIR}"
  File "/oname=${EXENAME}" "${SRCEXE}"
  File "/oname=HiAgent.ico" "${ICON}"

  ; The shell, beside the engine rather than under it: the shell resolves
  ; hi-agent.exe from its own directory (AppPaths.EngineExe), which is what
  ; makes the pair need no configuration to find each other.
!ifdef SHELLDIR
  File /r "${SHELLDIR}/"
!endif

  ; Shortcuts (icon from the .ico, since neither exe carries one yet).
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "${APPDIR}\${LAUNCHER}" "" "${APPDIR}\HiAgent.ico"
  CreateShortcut "$DESKTOP\${APPNAME}.lnk"    "${APPDIR}\${LAUNCHER}" "" "${APPDIR}\HiAgent.ico"

  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\hi-agent" "InstallDir" "$INSTDIR"

  ; Add/Remove Programs entry.
  WriteRegStr   HKCU "${ARP_KEY}" "DisplayName"     "${APPNAME}"
  WriteRegStr   HKCU "${ARP_KEY}" "DisplayVersion"  "${VERSION}"
  WriteRegStr   HKCU "${ARP_KEY}" "Publisher"       "${PUBLISHER}"
  WriteRegStr   HKCU "${ARP_KEY}" "DisplayIcon"     "${APPDIR}\HiAgent.ico"
  WriteRegStr   HKCU "${ARP_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKCU "${ARP_KEY}" "UninstallString"      '"$INSTDIR\uninstall.exe"'
  WriteRegStr   HKCU "${ARP_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${ARP_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${ARP_KEY}" "NoRepair" 1

  ; EstimatedSize (KB) shown in Add/Remove Programs.
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${ARP_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
  ; Remove the program files + shortcuts + registry. User data and the managed
  ; runtime cache (under %LOCALAPPDATA%/%APPDATA% ProjectDirs) are intentionally
  ; left untouched -- uninstalling the app must not delete the user's life DB.
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"
  ; Only the subdirectory this installer created, never $INSTDIR itself.
  RMDir /r "${APPDIR}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "${ARP_KEY}"
  DeleteRegKey HKCU "Software\hi-agent"
SectionEnd
