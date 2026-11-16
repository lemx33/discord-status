@echo off
rem Runs DiscordStatus.exe in the background (it has no console window of its own).
rem Logs go to the status.log next to your settings (%APPDATA%\DiscordStatus).
call "%~dp0stop.bat" >nul 2>&1
if exist "%~dp0target\release\DiscordStatus.exe" (
    start "" "%~dp0target\release\DiscordStatus.exe"
) else (
    start "" "%~dp0dist\DiscordStatus.exe"
)