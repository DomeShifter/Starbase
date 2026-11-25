# Builds the Starbase MSI (Starbase.wxs) with the WiX Toolset.
#
#   build-msi.ps1 -MsiVersion 0.1.0.7 -BinDir bin -IconDir icons -Out dist\starbase-0.1.0-dev.7-x64-setup.msi
#
# BinDir holds starsheets.exe and stardocs.exe (CI's Windows builds); IconDir holds drive.ico,
# sheets.ico and docs.ico (make-ico.py). MsiVersion is numeric (a.b.c or a.b.c.d): Windows
# Installer compares only a.b.c, so development builds of one release replace each other.
#
# WiX v7 asks every user to accept its licence (the Open Source Maintenance Fee EULA: only
# organisations with more than $10,000 a year in revenue must sponsor WiX; Starbase has none).
# -acceptEula wix7 records that acceptance for this build (docs/research/windows-installer.md).
param(
  [Parameter(Mandatory)] [string] $MsiVersion,
  [Parameter(Mandatory)] [string] $BinDir,
  [Parameter(Mandatory)] [string] $IconDir,
  [Parameter(Mandatory)] [string] $Out,
  [string] $WixVersion = "7.0.1"
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")

if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
  dotnet tool install --global wix --version $WixVersion
  if ($LASTEXITCODE -ne 0) { throw "couldn't install WiX $WixVersion" }
  $env:PATH += ";$env:USERPROFILE\.dotnet\tools"
}
wix --version

foreach ($f in "starsheets.exe", "stardocs.exe") {
  if (-not (Test-Path (Join-Path $BinDir $f))) { throw "missing $BinDir\$f" }
}
foreach ($f in "drive.ico", "sheets.ico", "docs.ico") {
  if (-not (Test-Path (Join-Path $IconDir $f))) { throw "missing $IconDir\$f" }
}
New-Item -ItemType Directory -Force (Split-Path -Parent $Out) | Out-Null

wix build -arch x64 -acceptEula wix7 `
  -d "Version=$MsiVersion" `
  -d "BinDir=$((Resolve-Path $BinDir).Path)" `
  -d "IconDir=$((Resolve-Path $IconDir).Path)" `
  -d "DocDir=$($root.Path)" `
  (Join-Path $PSScriptRoot "Starbase.wxs") -o $Out
if ($LASTEXITCODE -ne 0) { throw "wix build failed ($LASTEXITCODE)" }
Get-Item $Out | Select-Object Name, Length
