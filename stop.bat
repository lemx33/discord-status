@echo off
rem Stops the running copy of DiscordStatus.exe (not other programs).
rem Asks it to quit first so it can remove its tray icon, then force-stops it if needed.
powershell -NoProfile -Command "$p = Get-Process DiscordStatus -ErrorAction SilentlyContinue; if ($p) { try { [void][System.Threading.EventWaitHandle]::OpenExisting('Local\DiscordStatusPresenceStop').Set() } catch {}; $p.WaitForExit(6000); if (Get-Process DiscordStatus -ErrorAction SilentlyContinue) { Stop-Process -Name DiscordStatus -Force } }"