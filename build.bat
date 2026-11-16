@echo off
rem Builds target\release\DiscordStatus.exe and stages dist\ with everything it needs.
rem No other toolchain is required: the Rust code has no system dependencies.
setlocal
cd /d "%~dp0"

cargo build --release || exit /b 1

if not exist "dist" mkdir dist
copy /y "target\release\DiscordStatus.exe" "dist\DiscordStatus.exe" >nul
copy /y "config.example.json" "dist\config.example.json" >nul
copy /y "icon.ico" "dist\icon.ico" >nul
copy /y "LICENSE" "dist\LICENSE" >nul

echo.
echo Built dist\DiscordStatus.exe