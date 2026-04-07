# RUNESH CLI installer for Windows
# Usage: irm https://raw.githubusercontent.com/mydrift-user/runesh/main/install.ps1 | iex
$ErrorActionPreference = "Stop"

$Repo       = if ($env:RUNESH_REPO)        { $env:RUNESH_REPO }        else { "mydrift-user/runesh" }
$Version    = if ($env:RUNESH_VERSION)     { $env:RUNESH_VERSION }     else { "latest" }
$InstallDir = if ($env:RUNESH_INSTALL_DIR) { $env:RUNESH_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "runesh\bin" }
$BinName    = "runesh"

function Say($msg) { Write-Host "[runesh] $msg" -ForegroundColor Cyan }
function Die($msg) { Write-Host "[runesh] error: $msg" -ForegroundColor Red; exit 1 }

$arch = if ([Environment]::Is64BitOperatingSystem) { "x86_64" } else { Die "32-bit Windows is not supported" }
$target = "$arch-pc-windows-msvc"

if ($Version -eq "latest") {
    try {
        $release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent" = "runesh-installer" }
        $Tag = $release.tag_name
    } catch { Die "could not resolve latest release: $_" }
} else {
    $Tag = $Version
}

$asset = "$BinName-$target.zip"
$url   = "https://github.com/$Repo/releases/download/$Tag/$asset"

Say "downloading $asset ($Tag)"
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("runesh-" + [guid]::NewGuid()))
try {
    $zipPath = Join-Path $tmp $asset
    Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing
    Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $src = Join-Path $tmp "$BinName-$target\$BinName.exe"
    Copy-Item $src (Join-Path $InstallDir "$BinName.exe") -Force
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Say "installed $InstallDir\$BinName.exe"

# Add to user PATH if missing
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not ($userPath -split ';' | Where-Object { $_ -eq $InstallDir })) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
    Say "added $InstallDir to user PATH (restart your shell)"
}

& (Join-Path $InstallDir "$BinName.exe") --version
