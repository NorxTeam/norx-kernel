[CmdletBinding()]
param(
    [ValidateSet('x86_64', 'aarch64')]
    [string]$Arch = 'x86_64',
    [int]$TimeoutSeconds = 60,
    [string]$Machine,
    [string]$Cpu,
    [string]$Esp,
    [string]$Vars,
    [string]$SerialLog,
    [string]$SerialDevice = 'stdio',
    [string]$StorageImage,
    [string]$Display = 'none',
    [string[]]$Marker = @(),
    [switch]$Interactive
)

$ErrorActionPreference = 'Stop'
$kernelRoot = Split-Path -Parent $PSScriptRoot

function Resolve-RepoPath([string]$Path) {
    if ([IO.Path]::IsPathRooted($Path)) {
        return [IO.Path]::GetFullPath($Path)
    }
    return [IO.Path]::GetFullPath((Join-Path $kernelRoot $Path))
}

function Quote-ProcessArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') {
        return $Value
    }
    return '"' + $Value.Replace('"', '\"') + '"'
}

if ($TimeoutSeconds -lt 1) {
    throw 'TimeoutSeconds must be positive'
}

if (-not $Esp) {
    if ($Arch -eq 'x86_64') {
        $Esp = 'build\x86_64\esp'
    } elseif (Test-Path (Resolve-RepoPath 'build\aarch64-current\esp')) {
        $Esp = 'build\aarch64-current\esp'
    } else {
        $Esp = 'build\lifecycle-aarch64\esp'
    }
}
if (-not $Vars) {
    if ($Arch -eq 'x86_64') {
        $Vars = 'build\x86_64\nsh-smoke-vars.fd'
        if (-not (Test-Path (Resolve-RepoPath $Vars))) {
            $Vars = 'build\x86_64\edk2-i386-vars.fd'
        }
    } elseif (Test-Path (Resolve-RepoPath 'build\aarch64-current\vars.fd')) {
        $Vars = 'build\aarch64-current\vars.fd'
    } else {
        $Vars = 'build\lifecycle-aarch64\final-vars.fd'
    }
}

$Esp = Resolve-RepoPath $Esp
$Vars = Resolve-RepoPath $Vars
if (-not (Test-Path $Esp -PathType Container)) {
    throw "ESP directory not found: $Esp (build it with scripts/run.sh or pass -Esp)"
}
if (-not (Test-Path $Vars -PathType Leaf)) {
    throw "UEFI vars file not found: $Vars (pass -Vars)"
}
if ($StorageImage) {
    if ($Arch -ne 'x86_64') {
        throw 'StorageImage smoke is supported only on x86_64 virtio-blk'
    }
    $StorageImage = Resolve-RepoPath $StorageImage
    if (-not (Test-Path $StorageImage -PathType Leaf)) {
        throw "Storage image not found: $StorageImage"
    }
}

$qemuName = "qemu-system-$Arch.exe"
$qemuPath = if ($env:QEMU_BIN) {
    Resolve-RepoPath $env:QEMU_BIN
} else {
    Join-Path ${env:ProgramFiles} "qemu\$qemuName"
}
if (-not (Test-Path $qemuPath -PathType Leaf)) {
    $qemuCommand = Get-Command $qemuName -ErrorAction SilentlyContinue
    if ($qemuCommand) {
        $qemuPath = $qemuCommand.Source
    } else {
        throw "QEMU executable not found: $qemuPath (set QEMU_BIN)"
    }
}

$qemuShare = if ($env:QEMU_SHARE) {
    Resolve-RepoPath $env:QEMU_SHARE
} else {
    Join-Path ${env:ProgramFiles} 'qemu\share'
}
$firmware = if ($Arch -eq 'x86_64') { 'edk2-x86_64-code.fd' } else { 'edk2-aarch64-code.fd' }
$firmwarePath = Join-Path $qemuShare $firmware
if (-not (Test-Path $firmwarePath -PathType Leaf)) {
    throw "QEMU firmware not found: $firmwarePath (set QEMU_SHARE)"
}

$machineName = if ($Machine) { $Machine } elseif ($Arch -eq 'x86_64') { 'q35' } else { 'virt' }
$cpuName = if ($Cpu) { $Cpu } elseif ($Arch -eq 'x86_64') { 'max' } else { 'cortex-a57' }
$qemuArgs = @(
    '-M', $machineName,
    '-cpu', $cpuName,
    '-m', '256M',
    '-display', $Display,
    '-no-reboot',
    '-no-shutdown'
)
if ($Arch -eq 'x86_64') {
    $qemuArgs += @(
        '-vga', 'none',
        '-device', 'virtio-vga,edid=on,xres=1200,yres=800',
        '-device', 'qemu-xhci,id=xhci',
        '-netdev', 'user,id=net0',
        '-device', 'virtio-net-pci,netdev=net0,disable-modern=on'
    )
} else {
    $qemuArgs += @('-device', 'ramfb')
}
$qemuArgs += @(
    '-serial', $SerialDevice,
    '-drive', "if=pflash,format=raw,readonly=on,file=$firmwarePath",
    '-drive', "if=pflash,format=raw,file=$Vars",
    '-drive', "format=raw,file=fat:rw:$Esp"
)
if ($StorageImage) {
    $qemuArgs += @(
        '-drive', "format=raw,file=$StorageImage,if=none,id=storage",
        '-device', 'virtio-blk-pci,drive=storage'
    )
}

if ($Interactive) {
    Write-Host "Launching QEMU $Arch interactively from $Esp"
    $argumentLine = ($qemuArgs | ForEach-Object { Quote-ProcessArgument $_ }) -join ' '
    $process = Start-Process -FilePath $qemuPath -ArgumentList $argumentLine -NoNewWindow -Wait -PassThru
    exit $process.ExitCode
}

if (-not $SerialLog) {
    $SerialLog = "build\qemu-$Arch-boot.log"
}
$SerialLog = Resolve-RepoPath $SerialLog
$logDirectory = Split-Path -Parent $SerialLog
New-Item -ItemType Directory -Force $logDirectory | Out-Null
$stderrLog = "$SerialLog.stderr"
Remove-Item -LiteralPath $SerialLog, $stderrLog -Force -ErrorAction SilentlyContinue

$argumentLine = ($qemuArgs | ForEach-Object { Quote-ProcessArgument $_ }) -join ' '
$process = Start-Process -FilePath $qemuPath -ArgumentList $argumentLine -WindowStyle Hidden `
    -RedirectStandardOutput $SerialLog -RedirectStandardError $stderrLog -PassThru
$completed = $process.WaitForExit($TimeoutSeconds * 1000)
if (-not $completed) {
    Stop-Process -Id $process.Id -Force
}

$output = Get-Content -LiteralPath $SerialLog -Raw -ErrorAction SilentlyContinue
if (-not $completed) {
    Write-Host "QEMU $Arch timed out after ${TimeoutSeconds}s; serial log=$SerialLog"
    exit 124
}
$missing = @($Marker | Where-Object { $output -notlike "*$_*" })
if ($missing.Count -gt 0) {
    Write-Error ("Missing QEMU markers: " + ($missing -join ', '))
    exit 1
}
Write-Host "QEMU $Arch exited code=$($process.ExitCode); serial log=$SerialLog"
exit $process.ExitCode
