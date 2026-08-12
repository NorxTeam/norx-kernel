[CmdletBinding()]
param(
    [ValidateSet('x86_64', 'aarch64')]
    [string]$Arch = 'x86_64'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$rootfs = Join-Path $root '..\test-rootfs'
$target = if ($Arch -eq 'x86_64') { 'x86_64-unknown-none' } else { 'aarch64-unknown-uefi' }
$kernelName = if ($Arch -eq 'x86_64') { 'norx.elf' } else { 'norx.efi' }
$buildName = if ($Arch -eq 'x86_64') { 'norx_kernel' } else { 'norx_kernel.efi' }
$image = Join-Path $root "target\$target\release\$buildName"
$espName = if ($Arch -eq 'aarch64') { 'aarch64-current' } else { $Arch }
$destination = Join-Path $root "build\$espName\esp\boot\$kernelName"
$efiDestination = if ($Arch -eq 'aarch64') {
    Join-Path $root 'build\aarch64-current\esp\EFI\BOOT\BOOTAA64.EFI'
} else {
    $null
}

$saved = @{}
foreach ($name in 'ROOTFS_PATH', 'RUN_NSH_SMOKE', 'RUN_USERCTL_SMOKE', 'REQUIRE_USERSPACE_FIXTURE') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    $env:ROOTFS_PATH = (Resolve-Path $rootfs).Path
    $env:RUN_NSH_SMOKE = '1'
    $env:RUN_USERCTL_SMOKE = '1'
    $env:REQUIRE_USERSPACE_FIXTURE = '1'
    & cargo +nightly build --release --target $target
    if ($LASTEXITCODE -ne 0) {
        throw "kernel build failed for $Arch"
    }
} finally {
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name])
    }
}

New-Item -ItemType Directory -Force (Split-Path -Parent $destination) | Out-Null
Copy-Item -LiteralPath $image -Destination $destination -Force
if ($efiDestination) {
    New-Item -ItemType Directory -Force (Split-Path -Parent $efiDestination) | Out-Null
    Copy-Item -LiteralPath $image -Destination $efiDestination -Force
}
Write-Host "userctl kernel image updated: $destination"
