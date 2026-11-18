#Requires -Version 7.4
<#
.SYNOPSIS
Switches SteamVR's null driver on or off, and starts or stops SteamVR, so an OpenXR session runs with
no headset connected.

.DESCRIPTION
The null driver is a headset that exists only in software. With it SteamVR starts with no headset, and
the headset screen of the launch app can be run and checked on a PC alone.

-Enable writes the keys below into SteamVR's user settings file and records each key's value from
before in a state file beside this repo's build output. -Disable puts every recorded value back and
removes each key that did not exist. A key the state file does not hold keeps its value. Both refuse
to run while SteamVR runs, since SteamVR rewrites its settings file when it exits.

-Start starts SteamVR once the null driver is on and ends its room setup, which takes the scene from
any other application. -Stop ends SteamVR's processes, which do not close on a window close request.
Neither reads nor writes the settings file.

A check runs -Enable, -Start, the trial, -Stop, then -Disable.

.PARAMETER Enable
Switch the null driver on.

.PARAMETER Disable
Put the settings back as they were before -Enable.

.PARAMETER Start
Start SteamVR with the null driver on.

.PARAMETER Stop
End SteamVR's processes.

.PARAMETER SettingsFile
SteamVR's user settings file. The default is the file of the Steam folder the registry names.

.PARAMETER StateFile
The record of the values from before -Enable. The default is dist/steamvr-null-state.json in this repo.
#>
[CmdletBinding(DefaultParameterSetName = 'Enable')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Enable')] [switch] $Enable,
    [Parameter(Mandatory, ParameterSetName = 'Disable')] [switch] $Disable,
    [Parameter(Mandatory, ParameterSetName = 'Start')] [switch] $Start,
    [Parameter(Mandatory, ParameterSetName = 'Stop')] [switch] $Stop,
    [Parameter(ParameterSetName = 'Enable')] [Parameter(ParameterSetName = 'Disable')] [string] $SettingsFile,
    [Parameter(ParameterSetName = 'Enable')] [Parameter(ParameterSetName = 'Disable')]
    [Parameter(ParameterSetName = 'Start')] [string] $StateFile
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The keys of the null driver recipe. requireHmd, forcedDriver and activateMultipleDrivers make SteamVR
# take the null headset. pauseCompositorOnStandby keeps a headset nobody wears from pausing the session.
# enableDashboard keeps SteamVR's dashboard from covering half of the launch app's screen.
$Keys = [ordered]@{
    'driver_null.enable'             = $true
    'steamvr.requireHmd'             = $false
    'steamvr.forcedDriver'           = 'null'
    'steamvr.activateMultipleDrivers' = $true
    'power.pauseCompositorOnStandby' = $false
    'dashboard.enableDashboard'      = $false
}
$Absent = '<absent>'
$SteamVrProcesses = 'vrmonitor', 'vrserver', 'vrcompositor', 'vrdashboard', 'vrwebhelper', 'steamvr_room_setup'
$CompositorTimeout = [timespan]::FromSeconds(60)
# SteamVR started its room setup about 3 s after the compositor in the measured runs.
$RoomSetupWindow = [timespan]::FromSeconds(20)
$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $StateFile) { $StateFile = Join-Path $RepoRoot 'dist/steamvr-null-state.json' }

function Get-SteamFolder {
    (Get-ItemProperty 'HKCU:\Software\Valve\Steam').SteamPath
}

if ($Start) {
    if (-not (Test-Path $StateFile)) { throw "The null driver is not on ($StateFile does not exist). Run -Enable first." }
    $vrStartup = Join-Path (Get-SteamFolder) 'steamapps/common/SteamVR/bin/win64/vrstartup.exe'
    if (-not (Test-Path $vrStartup)) { throw "SteamVR's start program $vrStartup does not exist." }
    Start-Process -FilePath $vrStartup
    $deadline = [datetime]::Now + $CompositorTimeout
    while (-not (Get-Process vrcompositor -ErrorAction SilentlyContinue)) {
        if ([datetime]::Now -ge $deadline) { throw "SteamVR's compositor did not start within $($CompositorTimeout.TotalSeconds) s." }
        Start-Sleep -Seconds 1
    }
    # The room setup takes the scene, and SteamVR then ends the session of every other application.
    $ended = 0
    $deadline = [datetime]::Now + $RoomSetupWindow
    while ([datetime]::Now -lt $deadline) {
        $roomSetup = @(Get-Process steamvr_room_setup -ErrorAction SilentlyContinue)
        $roomSetup | Stop-Process -Force
        $ended += $roomSetup.Count
        Start-Sleep -Seconds 1
    }
    "SteamVR runs. $ended room setup process(es) ended."
    return
}
if ($Stop) {
    # SteamVR's processes do not close on a window close request, so they are ended.
    $running = @(Get-Process $SteamVrProcesses -ErrorAction SilentlyContinue)
    $running | Stop-Process -Force
    "$($running.Count) SteamVR process(es) ended."
    return
}

if (-not $SettingsFile) { $SettingsFile = Join-Path (Get-SteamFolder) 'config/steamvr.vrsettings' }
if (-not (Test-Path $SettingsFile)) { throw "SteamVR's settings file $SettingsFile does not exist." }
$running = Get-Process vrserver, vrmonitor, vrcompositor -ErrorAction SilentlyContinue
if ($running) { throw "SteamVR runs ($($running.Name -join ', ')). Close it first, since it rewrites its settings file when it exits." }

$settings = Get-Content $SettingsFile -Raw | ConvertFrom-Json -AsHashtable

if ($Enable) {
    if (Test-Path $StateFile) { throw "The null driver is already on ($StateFile exists). Run -Disable first." }
    $before = [ordered]@{}
    foreach ($key in $Keys.Keys) {
        $section, $name = $key.Split('.')
        if (-not $settings.ContainsKey($section)) { $settings[$section] = @{} }
        $before[$key] = $settings[$section].ContainsKey($name) ? $settings[$section][$name] : $Absent
        $settings[$section][$name] = $Keys[$key]
    }
    New-Item -ItemType Directory -Force (Split-Path -Parent $StateFile) | Out-Null
    $before | ConvertTo-Json | Set-Content $StateFile
    $settings | ConvertTo-Json -Depth 20 | Set-Content $SettingsFile
    "null driver on. The values from before are in $StateFile"
}
elseif ($Disable) {
    if (-not (Test-Path $StateFile)) { throw "The null driver is not on ($StateFile does not exist)." }
    $before = Get-Content $StateFile -Raw | ConvertFrom-Json -AsHashtable
    foreach ($key in $before.Keys) {
        $section, $name = $key.Split('.')
        if (-not $settings.ContainsKey($section)) { continue }
        if ($before[$key] -is [string] -and $before[$key] -eq $Absent) { $settings[$section].Remove($name) }
        else { $settings[$section][$name] = $before[$key] }
        if ($settings[$section].Count -eq 0) { $settings.Remove($section) }
    }
    $settings | ConvertTo-Json -Depth 20 | Set-Content $SettingsFile
    # The state file is this script's own record, so it is deleted and not moved to the Recycle Bin.
    Remove-Item $StateFile
    'null driver off. The settings are as they were before.'
}
