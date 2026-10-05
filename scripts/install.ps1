# Install Veyra on Windows from GitHub Releases, verified against SHA256SUMS.
#   irm https://raw.githubusercontent.com/iamzayn19/veyra/main/scripts/install.ps1 | iex
$ErrorActionPreference = "Stop"
$Repo = "iamzayn19/veyra"
$InstallDir = if ($env:VEYRA_INSTALL_DIR) { $env:VEYRA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\veyra" }
$Version = $env:VEYRA_VERSION
if (-not $Version) {
  $Version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name.TrimStart("v")
}
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") { throw "No prebuilt Veyra for $env:PROCESSOR_ARCHITECTURE. Build from source: https://github.com/$Repo" }
$Asset = "veyra-v$Version-x86_64-pc-windows-msvc.zip"
$Base = "https://github.com/$Repo/releases/download/v$Version"
$Tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("veyra-" + [guid]::NewGuid()))
try {
  Invoke-WebRequest "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
  Invoke-WebRequest "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp "SHA256SUMS")
  $Line = Get-Content (Join-Path $Tmp "SHA256SUMS") | Where-Object { $_ -match "\s$([regex]::Escape($Asset))$" }
  if (-not $Line) { throw "$Asset is not listed in SHA256SUMS" }
  $Expected = ($Line -split "\s+")[0].ToLower()
  $Actual = (Get-FileHash (Join-Path $Tmp $Asset) -Algorithm SHA256).Hash.ToLower()
  if ($Expected -ne $Actual) { throw "Checksum mismatch for $Asset" }
  Expand-Archive (Join-Path $Tmp $Asset) -DestinationPath $Tmp -Force
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  $Exe = Get-ChildItem -Path $Tmp -Recurse -Filter veyra.exe | Select-Object -First 1
  Copy-Item $Exe.FullName (Join-Path $InstallDir "veyra.exe") -Force
  Write-Host "Installed veyra $Version to $InstallDir (sha256 verified)"
  if (-not ($env:PATH -split ";" | Where-Object { $_ -eq $InstallDir })) {
    Write-Host "Add $InstallDir to your PATH to run 'veyra'."
  }
} finally {
  Remove-Item -Recurse -Force $Tmp
}
