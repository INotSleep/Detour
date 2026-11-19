@echo off
rem Runs the MCP server of the Detour installed here, so the core it starts knows the
rem Detour window; the detour.exe in this bundle only when Detour is not installed.
setlocal
set "D="
if exist "%LOCALAPPDATA%\Detour\detour.exe" set "D=%LOCALAPPDATA%\Detour\detour.exe"
if not defined D if exist "%ProgramFiles%\Detour\detour.exe" set "D=%ProgramFiles%\Detour\detour.exe"
if not defined D for /f "delims=" %%i in ('where detour.exe 2^>nul') do if not defined D set "D=%%i"
if not defined D set "D=%~dp0win32\detour.exe"
"%D%" mcp
exit /b %ERRORLEVEL%
