@echo off
setlocal

set "INSTALLER=%~dp0install_op_amp_lab_profile.ps1"

if not exist "%INSTALLER%" (
  echo Installation failed.
  echo.
  echo Keep install_op_amp_lab_profile.bat and
  echo install_op_amp_lab_profile.ps1 in the same folder, then try again.
  set "INSTALL_EXIT=1"
  goto :finish
)

echo Installing Op Amp Lab for the current Windows user...
echo.
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%INSTALLER%" %*
set "INSTALL_EXIT=%ERRORLEVEL%"

:finish
echo.
if "%INSTALL_EXIT%"=="0" (
  echo Installation finished successfully.
) else (
  echo Installation did not complete. Review the message above.
)

if not defined WVU_PROFILE_INSTALLER_NO_PAUSE pause
exit /b %INSTALL_EXIT%
