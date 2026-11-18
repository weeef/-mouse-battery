@echo off
rem Prints what the app sees: receivers found, raw reports and the battery reading.
cd /d "%~dp0"
set "PY=python"
where py >nul 2>nul && set "PY=py"
%PY% -m pip install -q -r requirements.txt
%PY% mouse_battery.pyw --debug
pause
