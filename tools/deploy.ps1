#Requires -Version 7.4
<#
.SYNOPSIS
Builds the mod, stages it in dist/, and installs the two game DLLs into the game's bin folder.
.DESCRIPTION
dist/ holds the launch app, the debug viewer and the trial runner at paths that stay the same across builds and
profiles, so a Steam "Add a Non-Steam Game" entry keeps working. dist/game-bin/ holds the files
the game's bin folder receives. A normal Steam launch stays unmodded while the DLLs are installed.
.PARAMETER Configuration
The cargo profile. A debug build carries the allocation guard. A release build gives headset frame times.
.PARAMETER GameBin
The game's bin folder. Steam's library list supplies it when the parameter is omitted.
.PARAMETER StageOnly
Builds and fills dist/ without touching the game folder.
.PARAMETER Remove
Moves the installed DLLs from the game's bin folder to the Recycle Bin, then exits.
#>
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string] $Configuration = 'debug',
    [string] $GameBin = '',
    [switch] $StageOnly,
    [switch] $Remove
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$SteamAppId = '1086940'
$GameFolder = 'Baldurs Gate 3'
$ProxyDll = 'version.dll'
$CoreDll = 'arcane_eye_core.dll'
$StagedApps = @('arcane-eye-vr.exe', 'viewer.exe', 'trial.exe')
$RepoRoot = Split-Path -Parent $PSScriptRoot
$StageRoot = Join-Path $RepoRoot 'dist'
$StageGameBin = Join-Path $StageRoot 'game-bin'

function Find-SteamGameBin {
    [OutputType([string])]
    param()
    $steamKey = 'HKCU:\Software\Valve\Steam'
    if (-not (Test-Path $steamKey)) { throw 'Steam is not installed for this user. Pass -GameBin.' }
    $libraryList = Join-Path (Get-ItemProperty $steamKey).SteamPath 'steamapps/libraryfolders.vdf'
    $libraries = Select-String -Path $libraryList -Pattern '"path"\s+"(.+)"' |
        ForEach-Object { $_.Matches[0].Groups[1].Value -replace '\\\\', '\' }
    foreach ($library in $libraries) {
        if (Test-Path (Join-Path $library "steamapps/appmanifest_$SteamAppId.acf")) {
            return Join-Path $library "steamapps/common/$GameFolder/bin"
        }
    }
    throw "No Steam library holds app $SteamAppId. Pass -GameBin."
}

function Get-GameBin {
    [OutputType([string])]
    param([string] $Override)
    $bin = if ($Override) { $Override } else { Find-SteamGameBin }
    if (-not (Test-Path (Join-Path $bin 'bg3_dx11.exe'))) { throw "'$bin' holds no bg3_dx11.exe, so it is not the game's bin folder." }
    return (Resolve-Path $bin).Path
}

function Assert-GameStopped {
    # Windows locks a loaded DLL, and both game exes load version.dll from bin.
    $running = @(Get-Process -Name 'bg3_dx11', 'bg3' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) { throw 'The game is running and holds the DLLs open. Close it first.' }
}

function Assert-StagedAppsStopped {
    # Windows locks a running exe, so an app started from dist/ blocks the copy.
    $stagedPaths = $StagedApps | ForEach-Object { Join-Path $StageRoot $_ }
    $running = @(Get-Process | Where-Object { $_.Path -in $stagedPaths })
    if ($running.Count -gt 0) { throw "Close $($running.Name -join ', ') first. It runs from $StageRoot." }
}

function Assert-ProxyOwned {
    param([string] $Bin)
    # Our proxy never sits in bin without the core, so a lone version.dll belongs to another mod.
    $proxyPresent = Test-Path (Join-Path $Bin $ProxyDll)
    $corePresent = Test-Path (Join-Path $Bin $CoreDll)
    if ($proxyPresent -and -not $corePresent) { throw "'$Bin' holds a $ProxyDll from another mod. Move it away first." }
}

function Invoke-Build {
    $cargoArgs = @('build', '--locked', '-p', 'proxy', '-p', 'arcane-eye-core', '-p', 'launcher', '-p', 'viewer', '-p', 'trial')
    if ($Configuration -eq 'release') { $cargoArgs += '--release' }
    cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed.' }
}

function Copy-BuildToStage {
    # cargo metadata honors CARGO_TARGET_DIR and config overrides of the target folder.
    $targetDir = (cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json).target_directory
    $buildDir = Join-Path $targetDir $Configuration
    New-Item -ItemType Directory -Force $StageGameBin | Out-Null
    foreach ($name in $StagedApps) { Copy-Item (Join-Path $buildDir $name) $StageRoot -Force }
    foreach ($name in $CoreDll, $ProxyDll) { Copy-Item (Join-Path $buildDir $name) $StageGameBin -Force }
}

function Install-ToGame {
    param([string] $Bin)
    # The core goes first, so a failed copy never leaves the proxy in bin without its core.
    foreach ($name in $CoreDll, $ProxyDll) { Copy-Item (Join-Path $StageGameBin $name) $Bin -Force }
}

function Uninstall-FromGame {
    param([string] $Bin)
    Add-Type -AssemblyName Microsoft.VisualBasic
    # The proxy goes first, for the same reason the install copies it last.
    $installed = @($ProxyDll, $CoreDll | ForEach-Object { Join-Path $Bin $_ } | Where-Object { Test-Path $_ })
    if ($installed.Count -eq 0) { Write-Host "Nothing to remove in $Bin" }
    foreach ($path in $installed) {
        [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($path, 'OnlyErrorDialogs', 'SendToRecycleBin')
        Write-Host "Moved to the Recycle Bin: $path"
    }
}

Push-Location $RepoRoot
try {
    if ($Remove) {
        $bin = Get-GameBin -Override $GameBin
        Assert-GameStopped
        Assert-ProxyOwned $bin
        Uninstall-FromGame $bin
        return
    }

    # Every precondition runs before the build, so a blocked copy fails in seconds.
    Assert-StagedAppsStopped
    if (-not $StageOnly) {
        $bin = Get-GameBin -Override $GameBin
        Assert-GameStopped
        Assert-ProxyOwned $bin
    }

    Invoke-Build
    Copy-BuildToStage
    Write-Host "Staged ($Configuration): $StageRoot"
    if ($StageOnly) { return }

    Install-ToGame $bin
    Write-Host "Installed: $bin"
    Write-Host "Launch:    $(Join-Path $StageRoot 'arcane-eye-vr.exe') --mode desktop-stereo"
    Write-Host "View:      $(Join-Path $StageRoot 'viewer.exe')"
    Write-Host "Trial:     $(Join-Path $StageRoot 'trial.exe') --name <name>"
}
finally {
    Pop-Location
}
