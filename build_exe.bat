@echo off
rem Builds dist\MouseBattery.exe. Needs Rust (https://rustup.rs) with the MSVC toolchain.
cd /d "%~dp0"
cargo build --release || (echo. & echo Build failed. & pause & exit /b 1)
if not exist dist mkdir dist
copy /y target\release\MouseBattery.exe dist\MouseBattery.exe >nul || (echo. & echo Could not copy the exe. Quit Mouse Battery first, then run this again. & pause & exit /b 1)
echo.
echo Done: dist\MouseBattery.exe
pause
