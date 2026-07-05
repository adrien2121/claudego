# install.ps1

# Stop on errors
$ErrorActionPreference = "Stop"

# GitHub repository
$Repo = "adrien2121/claudego"
$BinaryName = "claudego"

# Installation directory
$InstallDir = "$env:USERPROFILE\.$BinaryName\bin"
$ExePath = Join-Path $InstallDir "$BinaryName.exe"

# --- Script ---

# 1. Get latest release info from GitHub API
Write-Host "Fetching latest $BinaryName release..."
$ReleaseUrl = "https://api.github.com/repos/$Repo/releases/latest"
$Release = Invoke-RestMethod -Uri $ReleaseUrl

# 2. Find the Windows asset (.zip)
$Asset = $Release.assets | Where-Object { $_.name -like "*windows*.zip" }
if ($null -eq $Asset) {
    Write-Error "Could not find a Windows release asset for $($Release.tag_name)."
    exit 1
}

$DownloadUrl = $Asset.browser_download_url
$FileName = $Asset.name
$TempZipPath = Join-Path $env:TEMP $FileName

# 3. Download the release asset
Write-Host "Downloading $($FileName)..."
Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZipPath

# 4. Create installation directory and extract the archive
Write-Host "Installing to $InstallDir..."
New-Item -Path $InstallDir -ItemType Directory -Force | Out-Null
Expand-Archive -Path $TempZipPath -DestinationPath $InstallDir -Force

# 5. Clean up the downloaded zip file
Remove-Item -Path $TempZipPath

# 6. Check if the installation directory is in the user's PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not ($UserPath -like "*$InstallDir*")) {
    Write-Host ""
    Write-Host "ACTION REQUIRED: To complete the installation, add the following directory to your PATH:" -ForegroundColor Yellow
    Write-Host "  $InstallDir" -ForegroundColor Cyan
    Write-Host "You will need to restart your terminal for the change to take effect."
} else {
    Write-Host ""
    Write-Host "$BinaryName has been installed successfully!" -ForegroundColor Green
    Write-Host "You may need to restart your terminal to use the '$BinaryName' command."
}