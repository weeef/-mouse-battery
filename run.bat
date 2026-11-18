@echo off
rem Installs the requirements (first time only) and starts Mouse Battery.
cd /d "%~dp0"
set "PY=python"
set "PYW=pythonw"
where py >nul 2>nul && (set "PY=py" & set "PYW=pyw")
%PY% -m pip install -q -r requirements.txt || (echo. & echo Could not run Python. Install it from python.org and tick "Add python.exe to PATH". & pause & exit /b 1)
start "" %PYW% mouse_battery.pyw
