@echo off
rem Runs cargo for Quota from a path without spaces.
rem
rem The windows-gnu toolchain's dlltool fails when the project path contains
rem a space (for example "Better Projects"). In that case this script makes
rem a junction <drive>:\quota that points at this project and runs
rem cargo from there. A junction is only a pointer: deleting it deletes
rem nothing in the project, and this script recreates it when it is missing.
rem The build output still lands in this project's target\ folder.
rem
rem Usage:  scripts\cargo build --release
rem         scripts\cargo test --workspace
setlocal
for %%I in ("%~dp0..") do set "PROJ=%%~fI"

if "%PROJ%"=="%PROJ: =%" (
    rem No space in the path: no junction needed.
    cd /d "%PROJ%" || exit /b 1
    cargo %*
    exit /b %ERRORLEVEL%
)

set "LINK=%~d0\quota"
if not exist "%LINK%\Cargo.toml" (
    if exist "%LINK%" rmdir "%LINK%"
    mklink /J "%LINK%" "%PROJ%" >nul || (
        echo Could not create the junction %LINK%. 1>&2
        exit /b 1
    )
)
cd /d "%LINK%" || exit /b 1
cargo %*
exit /b %ERRORLEVEL%
