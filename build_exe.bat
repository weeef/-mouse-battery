@echo off
rem Builds a standalone dist\MouseBattery.exe (no Python needed to run it).
cd /d "%~dp0"
set "PY=python"
where py >nul 2>nul && set "PY=py"
set "WORK=%TEMP%\mousebattery-build"
%PY% -m pip install -q -r requirements.txt pyinstaller
%PY% -m PyInstaller --noconfirm --clean --onefile --noconsole --name MouseBattery ^
    --workpath "%WORK%" --specpath "%WORK%" mouse_battery.pyw
echo.
echo Done: dist\MouseBattery.exe
pause
