#Requires -Version 7.4
<#
.SYNOPSIS
Runs the project's quality gates. Claude Code hooks, lefthook, and CI all call this one script,
so every gate runs the same commands with the same flags.
#>
[CmdletBinding()]
param(
    [ValidateSet('commit', 'full', 'nightly')]
    [string] $Suite = 'commit',
    [string[]] $Stage = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The one place the nightly toolchain date lives. Miri and AddressSanitizer need nightly.
$NightlyToolchain = 'nightly-2026-10-29'
$RepoRoot = Split-Path -Parent $PSScriptRoot
$SuppressionFile = Join-Path $RepoRoot 'suppressions.txt'

$SuiteStages = @{
    commit  = @('fmt', 'clippy', 'rules', 'suppressions', 'typos')
    full    = @('fmt', 'clippy', 'rules', 'suppressions', 'typos', 'deps', 'test', 'lua', 'powershell')
    nightly = @('miri')
}

$StageCommands = @{
    fmt        = { cargo fmt --all --check }
    clippy     = { cargo clippy --workspace --all-targets --all-features --keep-going --locked -- -D warnings }
    rules      = { ast-grep scan --off=suppression-inventory --off=unsafe-inventory --report-style short }
    typos      = { typos }
    deps       = { cargo deny --locked check; if ($LASTEXITCODE -eq 0) { cargo shear } }
    test       = { cargo nextest run --workspace --locked --no-tests=warn; if ($LASTEXITCODE -eq 0) { cargo test --workspace --doc --locked } }
    miri       = { cargo "+$NightlyToolchain" miri test --locked -p stereo-core }
    # The Lua linter is undecided. selene fails to parse Lua 5.3 operators.
    lua        = { stylua --check lua }
    # -EnableExit would end this script, so the finding count sets the exit code instead.
    powershell = {
        $settings = Join-Path $RepoRoot 'PSScriptAnalyzerSettings.psd1'
        $findings = @(foreach ($dir in 'tools', '.claude/hooks') { Invoke-ScriptAnalyzer -Path (Join-Path $RepoRoot $dir) -Recurse -Settings $settings })
        $findings | Format-Table -AutoSize | Out-String | Write-Host
        $global:LASTEXITCODE = if ($findings.Count -eq 0) { 0 } else { 1 }
    }
}

function Get-SuppressionList {
    # Line numbers are left out so an edit above a suppression does not change the inventory.
    $hits = ast-grep scan --filter suppression-inventory --json=stream | ForEach-Object { $_ | ConvertFrom-Json }
    return $hits | ForEach-Object { '{0} | {1}' -f ($_.file -replace '\\', '/'), ($_.text -replace '\s+', ' ' -replace '\( ', '(' -replace ' \)', ')') } | Sort-Object
}

function Test-SuppressionInventory {
    $current = @(Get-SuppressionList)
    $approved = @(if (Test-Path $SuppressionFile) { Get-Content $SuppressionFile | Where-Object { $_ -ne '' } })
    $added = @($current | Where-Object { $_ -notin $approved })
    $removed = @($approved | Where-Object { $_ -notin $current })
    foreach ($line in $added) { Write-Error -ErrorAction Continue "unapproved suppression: $line" }
    foreach ($line in $removed) { Write-Error -ErrorAction Continue "suppressions.txt lists a suppression that no longer exists: $line" }
    $global:LASTEXITCODE = if ($added.Count + $removed.Count -eq 0) { 0 } else { 1 }
}
$StageCommands['suppressions'] = { Test-SuppressionInventory }

# `pwsh -File` passes "fmt,clippy" as one string, so the list is split here.
$selectedStages = if ($Stage.Count -gt 0) { $Stage -split ',' } else { $SuiteStages[$Suite] }
$failedStages = [System.Collections.Generic.List[string]]::new()

Push-Location $RepoRoot
try {
    foreach ($name in $selectedStages) {
        if (-not $StageCommands.ContainsKey($name)) { throw "unknown stage '$name'. Known stages: $($StageCommands.Keys -join ', ')" }
        Write-Host "== $name"
        $global:LASTEXITCODE = 0
        & $StageCommands[$name]
        if ($LASTEXITCODE -ne 0) { $failedStages.Add($name) }
    }
}
finally {
    Pop-Location
}

if ($failedStages.Count -gt 0) {
    Write-Error -ErrorAction Continue "gate failed: $($failedStages -join ', ')"
    exit 1
}
exit 0
