# Installs, starts, upgrades and uninstalls the Starbase MSI the way a user would (per user, no
# administrator), and checks each step. Run on a clean Windows (CI's windows-latest).
#
#   test-msi.ps1 -Msi first.msi -Upgrade second.msi [-Logs dir]
#
# second.msi is the same build with a higher version: installing it must replace first.msi in
# place. Exits non-zero at the first check that fails.
param(
  [Parameter(Mandatory)] [string] $Msi,
  [Parameter(Mandatory)] [string] $Upgrade,
  [string] $Logs = $env:RUNNER_TEMP
)
$ErrorActionPreference = "Stop"
$Msi = (Resolve-Path $Msi).Path
$Upgrade = (Resolve-Path $Upgrade).Path
$dir = Join-Path $env:LOCALAPPDATA "Programs\Starbase"
$menu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$shortcuts = "Starbase Drive.lnk", "Starbase Sheets.lnk", "Starbase Docs.lnk"
$failed = 0

function Check([bool] $ok, [string] $what) {
  if ($ok) { Write-Host "ok   $what" } else { Write-Host "FAIL $what"; $script:failed++ }
}
function Msiexec([string] $what, [string[]] $arguments) {
  $p = Start-Process msiexec.exe -ArgumentList $arguments -Wait -PassThru
  Check ($p.ExitCode -eq 0) "$what (msiexec exit $($p.ExitCode))"
  if ($p.ExitCode -ne 0) { exit 1 }
}
function Value([string] $key, [string] $name) {
  $item = Get-ItemProperty -Path $key -ErrorAction SilentlyContinue
  if ($null -eq $item) { return $null }
  return $item.$name
}
# Installed Starbase products, from Windows Installer itself (per-user MSIs don't write the
# HKCU ...\Uninstall key the Settings page reads for other installers).
function Entries() {
  $wi = New-Object -ComObject WindowsInstaller.Installer
  @($wi.ProductsEx("", "", 7) | Where-Object { $_.InstallProperty("ProductName") -eq "Starbase" })
}
# Starts a program (or a shortcut), checks its process is still there after 15 s, stops it.
function Starts([string] $what, [string] $process, [string] $file, [string[]] $arguments = @()) {
  Get-Process $process -ErrorAction SilentlyContinue | Stop-Process -Force
  if ($arguments.Count) { Start-Process $file -ArgumentList $arguments } else { Start-Process $file }
  Start-Sleep -Seconds 15
  $running = Get-Process $process -ErrorAction SilentlyContinue
  Check ($null -ne $running) "$what is running 15 s later"
  $running | Stop-Process -Force
  Start-Sleep -Seconds 1
}

# The user's own default for .xlsx and .docx before (none on a clean runner): must not change.
$defaults = @{}
foreach ($ext in ".xlsx", ".docx", ".csv") { $defaults[$ext] = Value "HKCU:\Software\Classes\$ext" "(default)" }

Write-Host "== install"
Msiexec "install" @("/i", "`"$Msi`"", "/qn", "/l*v", "`"$Logs\install.log`"")
foreach ($f in "starsheets.exe", "stardocs.exe", "sheets.ico", "LICENSE-MIT.txt") { Check (Test-Path (Join-Path $dir $f)) "installed $dir\$f" }
foreach ($s in $shortcuts) { Check (Test-Path (Join-Path $menu $s)) "Start menu: $s" }
Check ((Entries).Count -eq 1) "one Starbase entry in Installed apps"
Check ($null -ne (Value "HKCU:\Software\Classes\.xlsx\OpenWithProgids" "Starbase.Sheets.Workbook")) ".xlsx offers Starbase Sheets under Open with"
Check ($null -ne (Value "HKCU:\Software\Classes\.odt\OpenWithProgids" "Starbase.Docs.Document")) ".odt offers Starbase Docs under Open with"
Check ($null -ne (Value "HKCU:\Software\RegisteredApplications" "Starbase Sheets")) "Starbase Sheets listed in Default apps"
foreach ($ext in $defaults.Keys) { Check ((Value "HKCU:\Software\Classes\$ext" "(default)") -eq $defaults[$ext]) "$ext default app unchanged" }
$version = (& (Join-Path $dir "starsheets.exe") --version 2>&1 | Out-String).Trim()
Check ($version -like "Starbase Sheets *") "starsheets --version: $version"
$version = (& (Join-Path $dir "stardocs.exe") --version 2>&1 | Out-String).Trim()
Check ($version -like "stardocs *") "stardocs --version: $version"

Write-Host "== launch"
Starts "Starbase Drive (Start menu)" "starsheets" (Join-Path $menu "Starbase Drive.lnk")
Starts "Starbase Docs (Start menu)" "stardocs" (Join-Path $menu "Starbase Docs.lnk")
# A CSV opened the way Explorer's "Open with" does: the registered command with the file.
$csv = Join-Path $env:RUNNER_TEMP "sample.csv"
Set-Content -Path $csv -Value "a,b`r`n1,2"
$command = Value "HKCU:\Software\Classes\Starbase.Sheets.TextData\shell\open\command" "(default)"
Check ($command -like "*starsheets.exe*") "CSV open command: $command"
Starts "Starbase Sheets with a CSV" "starsheets" (Join-Path $dir "starsheets.exe") @("`"$csv`"")

# The person's own files, which uninstalling must leave alone.
$docs = [Environment]::GetFolderPath("MyDocuments")
$mine = @((Join-Path $docs "Starbase\keep-me.txt"), (Join-Path $env:APPDATA "Starbase\keep-me.txt"))
foreach ($f in $mine) { New-Item -ItemType Directory -Force (Split-Path -Parent $f) | Out-Null; Set-Content $f "mine" }

Write-Host "== upgrade in place"
Msiexec "upgrade" @("/i", "`"$Upgrade`"", "/qn", "/l*v", "`"$Logs\upgrade.log`"")
$entries = Entries
Check ($entries.Count -eq 1) "still one Starbase entry after the upgrade (found $($entries.Count))"
Write-Host "     installed version: $($entries.DisplayVersion)"
Check (Test-Path (Join-Path $dir "starsheets.exe")) "programs still there after the upgrade"
foreach ($s in $shortcuts) { Check (Test-Path (Join-Path $menu $s)) "Start menu after the upgrade: $s" }
Starts "Starbase Sheets after the upgrade" "starsheets" (Join-Path $menu "Starbase Sheets.lnk")

Write-Host "== uninstall"
Msiexec "uninstall" @("/x", "`"$Upgrade`"", "/qn", "/l*v", "`"$Logs\uninstall.log`"")
Check (-not (Test-Path (Join-Path $dir "starsheets.exe"))) "programs removed"
foreach ($s in $shortcuts) { Check (-not (Test-Path (Join-Path $menu $s))) "Start menu entry removed: $s" }
Check ((Entries).Count -eq 0) "no Starbase entry left in Installed apps"
Check ($null -eq (Value "HKCU:\Software\Classes\.xlsx\OpenWithProgids" "Starbase.Sheets.Workbook")) "Open with entry removed"
Check ($null -eq (Value "HKCU:\Software\RegisteredApplications" "Starbase Sheets")) "Default apps entry removed"
foreach ($f in $mine) { Check (Test-Path $f) "kept the person's file $f" }

if ($failed) { Write-Host "$failed check(s) failed"; exit 1 }
Write-Host "all checks passed"
