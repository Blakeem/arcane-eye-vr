#Requires -Version 7.4
<#
.SYNOPSIS
Switches SteamVR's null driver on or off, so an OpenXR session runs with no headset connected.

.DESCRIPTION
The null driver is a headset that exists only in software. With it SteamVR starts with no headset, and
the headset screen of the launch app can be run and checked on a PC alone.

-Enable writes the keys below into SteamVR's user settings file and records each key's value from
before in a state file beside this repo's build output. -Disable puts every recorded value back and
removes each key that did not exist. Both refuse to run while SteamVR runs, since SteamVR rewrites its
settings file when it exits.

.PARAMETER Enable
Switch the null driver on.

.PARAMETER Disable
Put the settings back as they were before -Enable.

.PARAMETER SettingsFile
SteamVR's user settings file. The default is the file of the Steam folder the registry names.
#>
[CmdletBinding(DefaultParameterSetName = 'Enable')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Enable')] [switch] $Enable,
    [Parameter(Mandatory, ParameterSetName = 'Disable')] [switch] $Disable,
    [string] $SettingsFile
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The keys of the null driver recipe. requireHmd, forcedDriver and activateMultipleDrivers make SteamVR
# take the null headset. pauseCompositorOnStandby keeps a headset nobody wears from pausing the session.
$Keys = [ordered]@{
    'driver_null.enable'             = $true
    'steamvr.requireHmd'             = $false
    'steamvr.forcedDriver'           = 'null'
    'steamvr.activateMultipleDrivers' = $true
    'power.pauseCompositorOnStandby' = $false
}
$Absent = '<absent>'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$StateFile = Join-Path $RepoRoot 'dist/steamvr-null-state.json'

if (-not $SettingsFile) {
    $steam = (Get-ItemProperty 'HKCU:\Software\Valve\Steam').SteamPath
    $SettingsFile = Join-Path $steam 'config/steamvr.vrsettings'
}
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
