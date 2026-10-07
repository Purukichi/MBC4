# Builds MBC4 (VST3 + Standalone).
# JUCE's juceaide cannot handle the non-ASCII characters in this project's path,
# so the parent folder is mapped to a temporary drive letter (subst) for the build.
param(
    [string]$Config = "Release",
    [string]$Drive  = "M:",
    [switch]$Install   # copy the .vst3 to C:\Program Files\Common Files\VST3 (needs admin)
)

$ErrorActionPreference = "Stop"
$cmake  = "C:\Program Files\CMake\bin\cmake.exe"
$parent = Split-Path -Parent $PSScriptRoot
$name   = Split-Path -Leaf $PSScriptRoot

if (-not (Test-Path "$Drive\")) { subst $Drive $parent }
$src = "$Drive\$name"
$bld = "$src\build"

& $cmake -S $src -B $bld -G "Visual Studio 18 2026" -A x64
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cmake --build $bld --config $Config --parallel
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$vst3 = "$bld\MBC4_artefacts\$Config\VST3\MBC4.vst3"
Write-Host "VST3      : $vst3"
Write-Host "Standalone: $bld\MBC4_artefacts\$Config\Standalone\MBC4.exe"

if ($Install) {
    Copy-Item $vst3 "C:\Program Files\Common Files\VST3\" -Recurse -Force
    Write-Host "Installed to C:\Program Files\Common Files\VST3"
}
