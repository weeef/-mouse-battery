@echo off
rem Prints what the app sees: receivers found, raw reports and the battery reading.
cd /d "%~dp0"
if exist dist\MouseBattery.exe (
    dist\MouseBattery.exe --debug
) else (
    cargo run --release -- --debug
)
pause
