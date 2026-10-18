<#
  Assembles the payload and builds the MSI with WiX v5.

    ./build-msi.ps1 -Arch x64 -Version 0.1.0 `
      -AppDir ..\target\x86_64-pc-windows-msvc\release `
      -DriverDir ..\driver\x64\Release\PabulongMic

  Output: windows\out\Pabulong-<Version>-<Arch>.msi (unsigned).
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [string] $Version = '0.1.0',
  [Parameter(Mandatory)] [string] $AppDir,
  [Parameter(Mandatory)] [string] $DriverDir
)
$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\.."
$payload = Join-Path $root "out\payload-$Arch"
Remove-Item -Recurse -Force $payload -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$payload\driver" | Out-Null

Copy-Item "$AppDir\Pabulong.exe", "$AppDir\pabulong-setup.exe" $payload
foreach ($f in 'PabulongMic.sys', 'PabulongMic.inf', 'PabulongMic.cat') { Copy-Item (Join-Path $DriverDir $f) "$payload\driver" }
Copy-Item "$root\..\THIRD_PARTY_NOTICES.md" "$payload\THIRD_PARTY_NOTICES.txt"
Copy-Item "$root\..\LICENSE" "$payload\LICENSE.txt"

# The license dialog needs RTF.
$text = (Get-Content -Raw "$root\..\LICENSE") -replace '\\', '\\\\' -replace '\{', '\{' -replace '\}', '\}'
$body = ($text -split "`r?`n") -join "\par`r`n"
Set-Content -Encoding ascii "$payload\LICENSE.rtf" "{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18 $body}"

$msi = Join-Path $root "out\Pabulong-$Version-$Arch.msi"
wix build -arch $Arch -ext WixToolset.Util.wixext -ext WixToolset.UI.wixext `
  -d "Version=$Version" -d "PayloadDir=$payload" -o $msi "$root\installer\Pabulong.wxs"
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
Write-Host "MSI -> $msi"
