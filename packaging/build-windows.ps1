$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force dist | Out-Null
Compress-Archive -Force -Path target/release/ferrum.exe, README.md, LICENSE -DestinationPath "dist/Ferrum-$version-windows-x86_64.zip"
$nsis = 'C:\Program Files (x86)\NSIS\makensis.exe'
if (-not (Test-Path $nsis)) { throw 'Install NSIS before packaging the Windows installer.' }
& $nsis "/DVERSION=$version" packaging\windows\installer.nsi
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
