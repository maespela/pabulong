<#
  Builds RNNoise (BSD-3-Clause, https://github.com/xiph/rnnoise) as a static
  MSVC-compatible library, rnnoise.lib, for the shared Rust core's
  `rnnoise` feature. Pinned to the same upstream commit the macOS library
  was built from (see windows/README.md), and the trained model is checked
  against the SHA-256 that RNNoise itself publishes in model_version.

    ./build-rnnoise.ps1 -Arch x64   -Out ..\out\rnnoise\x64
    ./build-rnnoise.ps1 -Arch arm64 -Out ..\out\rnnoise\arm64

  Needs git and clang-cl / llvm-lib (bundled with Visual Studio's LLVM
  component and on GitHub's Windows runners).
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [string] $Out = "$PSScriptRoot\..\out\rnnoise\$Arch",
  [string] $Work = "$env:RUNNER_TEMP\rnnoise-src"
)
$ErrorActionPreference = 'Stop'

$Commit = '70f1d256acd4b34a572f999a05c87bf00b67730d'   # xiph/rnnoise main, 2025-02-22
$ModelSha256 = '0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37'

if (-not (Test-Path "$Work\.git")) {
  git clone --quiet https://github.com/xiph/rnnoise $Work
}
git -C $Work checkout --quiet $Commit
$version = (Get-Content "$Work\model_version").Trim()
if ($version -ne $ModelSha256) { throw "model_version is $version, expected $ModelSha256" }

$model = "$Work\rnnoise_data-$version.tar.gz"
if (-not (Test-Path $model)) {
  Invoke-WebRequest -Uri "https://media.xiph.org/rnnoise/models/rnnoise_data-$version.tar.gz" -OutFile $model
}
$actual = (Get-FileHash -Algorithm SHA256 $model).Hash.ToLower()
if ($actual -ne $ModelSha256) { throw "model checksum $actual does not match $ModelSha256" }
tar -xzf $model -C $Work

$sources = 'denoise', 'rnn', 'pitch', 'kiss_fft', 'celt_lpc', 'nnet', 'nnet_default', 'parse_lpcnet_weights', 'rnnoise_data', 'rnnoise_tables' |
  ForEach-Object { "$Work\src\$_.c" }
$target = if ($Arch -eq 'x64') { 'x86_64-pc-windows-msvc' } else { 'aarch64-pc-windows-msvc' }

New-Item -ItemType Directory -Force $Out | Out-Null
$objects = @()
foreach ($src in $sources) {
  $obj = Join-Path $Out ([IO.Path]::GetFileNameWithoutExtension($src) + '.obj')
  # -O2 matters: an unoptimised RNNoise costs ~13% of a core (see Scripts/vendor-rnnoise.sh).
  & clang-cl --target=$target /nologo /O2 /MD /Zl /DRNNOISE_BUILD /D_USE_MATH_DEFINES /I "$Work\include" /I "$Work\src" /c $src /Fo$obj
  if ($LASTEXITCODE -ne 0) { throw "clang-cl failed on $src" }
  $objects += $obj
}
& llvm-lib /nologo /out:"$Out\rnnoise.lib" @objects
if ($LASTEXITCODE -ne 0) { throw 'llvm-lib failed' }
Copy-Item "$Work\include\rnnoise.h" $Out
Write-Host "rnnoise.lib ($Arch) -> $Out"
