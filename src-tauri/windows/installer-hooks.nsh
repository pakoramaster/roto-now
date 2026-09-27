!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Checking required Roto Now models..."
  ReadEnvStr $0 "ROTO_NOW_SKIP_MODEL_SETUP"
  ${If} $0 == "1"
    DetailPrint "Model setup skipped for installer validation."
  ${Else}
    File /oname=$PLUGINSDIR\model-manifest.json "${__FILEDIR__}\..\model-manifest.json"
    File /oname=$PLUGINSDIR\install-required-models.ps1 "${__FILEDIR__}\install-required-models.ps1"
    nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\install-required-models.ps1" -ManifestPath "$PLUGINSDIR\model-manifest.json" -ModelRoot "$APPDATA\com.rotonow.desktop\models"'
    Pop $1
    ${If} $1 != 0
      MessageBox MB_ICONEXCLAMATION|MB_OK "Roto Now was installed, but its required AI models could not be downloaded. Open the app while online to retry General, or use Install Cutie High Detail in the video/GIF panel."
    ${EndIf}
  ${EndIf}
!macroend
