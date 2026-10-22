<#
  One-shot installer for workbuddy-pet (Windows 10/11 x64).

  Usage:
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -DeviceIp 192.168.1.100

  What it does:
    1. checks the toolchain (cargo / cmake / MSVC build tools / git)
    2. clones the official zectrix-local-push-sdk next to this repository
       (Cargo.toml points at ../zectrix-local-push-sdk, so the sibling layout
       is mandatory and this script creates it for you)
    3. builds and tests with CMAKE_POLICY_VERSION_MINIMUM=3.5
    4. verifies the produced exe
    5. optional: with -DeviceIp, checks the device, pushes one frame and
       enables autostart

  Exit codes: 0 ok | 1 prerequisites | 2 sdk | 3 build | 4 test | 5 verify | 6 device
#>
[CmdletBinding()]
param(
    [string]$DeviceIp = '',
    [switch]$SkipTests
)

$ErrorActionPreference = 'Continue'
# The exe prints UTF-8; without this, redirections and older consoles turn its
# Chinese output into mojibake, which hides the verification lines.
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch { }
$script:repo = Split-Path -Parent $PSScriptRoot
$script:sdk = Join-Path (Split-Path -Parent $script:repo) 'zectrix-local-push-sdk'
$script:sdkUrl = 'https://github.com/itopinion/zectrix-local-push-sdk.git'

function Step($text) { Write-Host ''; Write-Host "== $text ==" -ForegroundColor Cyan }
function Info($text) { Write-Host "   $text" }
function Fail($code, $text) {
    Write-Host ''
    Write-Host "FAILED: $text" -ForegroundColor Red
    exit $code
}
function Have($name) { return [bool](Get-Command $name -ErrorAction SilentlyContinue) }

Step '[1/5] toolchain'
Info "repository : $script:repo"
Info "sdk path   : $script:sdk"

# A fresh rustup / CMake install may not be on PATH in the shell that started
# this script (common when an installer just finished). Look in the default
# locations before declaring anything missing.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path (Join-Path $cargoBin 'cargo.exe')) -and -not (Have 'cargo')) {
    $env:Path = "$cargoBin;$env:Path"
}
$cmakeBin = Join-Path $env:ProgramFiles 'CMake\bin'
if ((Test-Path (Join-Path $cmakeBin 'cmake.exe')) -and -not (Have 'cmake')) {
    $env:Path = "$cmakeBin;$env:Path"
}

$missing = @()
if (-not (Have 'cargo')) { $missing += 'rustup (cargo/rustc)' }
if (-not (Have 'cmake')) { $missing += 'cmake' }
if (-not (Have 'git')) { $missing += 'git' }

# MSVC: the SDK compiles native Opus through CMake, so the C++ toolchain must exist.
$hasMsvc = $false
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path $vswhere) {
    $vcPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ($LASTEXITCODE -eq 0 -and $vcPath) { $hasMsvc = $true; Info "msvc       : $vcPath" }
}
if (-not $hasMsvc) { $missing += 'Visual Studio C++ Build Tools (MSVC)' }

if ($missing.Count -gt 0) {
    Write-Host ''
    Write-Host 'Missing prerequisites:' -ForegroundColor Yellow
    foreach ($item in $missing) { Write-Host "  - $item" }
    Write-Host ''
    Write-Host 'Install them (elevated PowerShell), then reopen the shell and rerun this script:'
    Write-Host '  winget install --id Rustlang.Rustup -e'
    Write-Host '  winget install --id Kitware.CMake -e'
    Write-Host '  winget install --id Git.Git -e'
    Write-Host '  winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
    Fail 1 'toolchain incomplete'
}

$rustInfo = (& rustc -vV) -join "`n"
if ($rustInfo -notmatch 'host: x86_64-pc-windows-msvc') {
    Info 'WARNING: rustc host is not x86_64-pc-windows-msvc.'
    Info '         Run: rustup toolchain install stable-x86_64-pc-windows-msvc'
    Info '              rustup default stable-x86_64-pc-windows-msvc'
}

Step '[2/5] official SDK at the sibling path'
if (Test-Path (Join-Path $script:sdk 'Cargo.toml')) {
    Info 'already present, reusing it (this script never modifies the SDK)'
} else {
    if (Test-Path $script:sdk) { Fail 2 "$script:sdk exists but is not the SDK (no Cargo.toml)" }
    Info "cloning $script:sdkUrl"
    & git clone $script:sdkUrl $script:sdk
    if ($LASTEXITCODE -ne 0) { Fail 2 'git clone of zectrix-local-push-sdk failed (network/proxy?)' }
}
if (-not (Test-Path (Join-Path $script:sdk 'Cargo.toml'))) { Fail 2 'SDK clone looks incomplete' }

Step '[3/5] build'
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
Push-Location $script:repo
& cargo build --locked --release
if ($LASTEXITCODE -ne 0) { Pop-Location; Fail 3 'cargo build failed (see messages above)' }

Step '[4/5] tests'
if ($SkipTests) {
    Info 'skipped (-SkipTests)'
} else {
    & cargo test --locked --release
    if ($LASTEXITCODE -ne 0) { Pop-Location; Fail 4 'cargo test failed (see messages above)' }
}
Pop-Location

Step '[5/5] verify'
$exe = Join-Path $script:repo 'target\release\workbuddy-pet.exe'
if (-not (Test-Path $exe)) { Fail 5 "exe not found: $exe" }
Info ("exe        : {0} ({1:N1} MB)" -f $exe, ((Get-Item $exe).Length / 1MB))
& $exe where
if ($LASTEXITCODE -ne 0) { Fail 5 'the exe cannot report its data directory' }

if ($DeviceIp -ne '') {
    Step 'device'
    Info "checking $DeviceIp"
    & $exe check --target $DeviceIp
    if ($LASTEXITCODE -ne 0) {
        Fail 6 'device check failed: same LAN? right IP? "Settings > Network sync > keep Wi-Fi in standby" on? another program holding the device?'
    }
    Info 'pushing one frame'
    # The firmware keeps the single connection for a few seconds after a clean
    # close (measured: Busy at t+0, accepted at t+8), so do not push back to
    # back with the check above.
    Start-Sleep -Seconds 8
    & $exe push --target $DeviceIp
    if ($LASTEXITCODE -ne 0) {
        Info 'device may still hold the lease, retrying once after 8s'
        Start-Sleep -Seconds 8
        & $exe push --target $DeviceIp
        if ($LASTEXITCODE -ne 0) { Fail 6 'push failed twice: device busy or unreachable' }
    }
    Info 'enabling autostart'
    & $exe autostart on --target $DeviceIp
    if ($LASTEXITCODE -ne 0) { Fail 6 'autostart failed' }
}

Write-Host ''
Write-Host 'DONE' -ForegroundColor Green
Info "exe     : $exe"
Info 'next    : start WorkBuddy, then run one of these:'
Info "            & `"$exe`" watch                          # follow tasks until you press Ctrl+C"
Info "            & `"$exe`" autostart on --target <IP>      # run in background at login"
Info '          (PowerShell needs the & before a quoted path; cmd does not)'
if ($DeviceIp -eq '') {
    Info 'device  : not touched. Re-run with -DeviceIp <IP> to check, push and enable autostart.'
}
exit 0
