# Build Laralah for Windows and package it as a .zip.
#
#   pwsh scripts/package-windows.ps1                              # host architecture
#   pwsh scripts/package-windows.ps1 -Target x86_64-pc-windows-msvc
#   pwsh scripts/package-windows.ps1 -Target aarch64-pc-windows-msvc
#
# Output: dist/Laralah_<version>_windows_<arch>.zip
param(
    [string]$Target = ""
)
$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

if (-not $Target) {
    $Target = ((rustc -vV) | Select-String '^host: ').ToString().Substring(6).Trim()
}
switch ($Target) {
    "x86_64-pc-windows-msvc" { $Arch = "x64" }
    "aarch64-pc-windows-msvc" { $Arch = "arm64" }
    default { throw "unsupported Windows target: $Target" }
}

$Version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value

cargo build --release --locked --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$Name = "Laralah_${Version}_windows_${Arch}"
$Staging = Join-Path ([System.IO.Path]::GetTempPath()) $Name
if (Test-Path $Staging) { Remove-Item -Recurse -Force $Staging }
New-Item -ItemType Directory -Path $Staging | Out-Null
Copy-Item "target/$Target/release/Laralah.exe" $Staging

New-Item -ItemType Directory -Force -Path dist | Out-Null
$Zip = "dist/$Name.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path (Join-Path $Staging "*") -DestinationPath $Zip
Remove-Item -Recurse -Force $Staging

Write-Host "Packaged: $Zip"
