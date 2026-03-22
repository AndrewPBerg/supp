#Requires -Version 5.1
<#
.SYNOPSIS
    Install supp on Windows.
.DESCRIPTION
    Downloads the latest supp release for Windows x86_64 and installs it.
.PARAMETER Version
    Specific version to install (e.g. "0.3.0"). Defaults to latest.
.PARAMETER InstallDir
    Installation directory. Defaults to ~/.supp/bin.
#>
param(
    [string]$Version,
    [string]$InstallDir
)

$ErrorActionPreference = "Stop"
$Repo = "AndrewPBerg/supp"
$Target = "x86_64-pc-windows-msvc"

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:USERPROFILE ".supp\bin"
}

# Get latest version from GitHub API
if (-not $Version) {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $release.tag_name -replace '^v', ''
}

Write-Host "Installing supp v$Version ($Target)..."

$ZipName = "supp-$Target.zip"
$Url = "https://github.com/$Repo/releases/download/v$Version/$ZipName"
$ChecksumUrl = "https://github.com/$Repo/releases/download/v$Version/SHA256SUMS"

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "supp-install-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TmpDir $ZipName
    $ChecksumPath = Join-Path $TmpDir "SHA256SUMS"

    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
    Invoke-WebRequest -Uri $ChecksumUrl -OutFile $ChecksumPath -UseBasicParsing

    # Verify checksum
    $expectedLine = (Get-Content $ChecksumPath | Where-Object { $_ -match $ZipName })
    if (-not $expectedLine) {
        Write-Error "Checksum entry not found for $ZipName"
        exit 1
    }
    $expectedHash = ($expectedLine -split '\s+')[0]
    $actualHash = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()
    if ($actualHash -ne $expectedHash) {
        Write-Error "Checksum mismatch: expected $expectedHash, got $actualHash"
        exit 1
    }
    Write-Host "Checksum verified."

    # Extract
    Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

    # Install
    if (-not (Test-Path $InstallDir)) {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    }
    Copy-Item (Join-Path $TmpDir "supp.exe") -Destination (Join-Path $InstallDir "supp.exe") -Force

    # Add to PATH if not already there
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
        Write-Host "Added $InstallDir to your PATH (restart your terminal to pick it up)."
    }

    Write-Host "supp v$Version installed to $InstallDir\supp.exe"
}
finally {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}
