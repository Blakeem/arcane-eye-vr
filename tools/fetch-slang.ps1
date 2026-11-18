#Requires -Version 7.4
<#
.SYNOPSIS
Downloads the Slang compiler release that shaders/slangc.toml pins, for the gfx build script.
.DESCRIPTION
The release zip goes to reference/downloads/ and is unpacked beside itself, where crates/gfx/build.rs
looks for bin/slangc.exe. The zip's SHA-256 must equal the pinned one before anything is unpacked.
A run that finds the compiler in place downloads nothing.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
$PinFile = Join-Path $RepoRoot 'shaders/slangc.toml'
$Downloads = Join-Path $RepoRoot 'reference/downloads'

function Get-PinValue {
    [OutputType([string])]
    param([string] $Key)
    $line = Select-String -Path $PinFile -Pattern "^$Key\s*=\s*`"(.+)`"" | Select-Object -First 1
    if (-not $line) { throw "'$PinFile' holds no '$Key' line." }
    return $line.Matches[0].Groups[1].Value
}

function Test-ZipHash {
    [OutputType([bool])]
    param([string] $Path, [string] $Expected)
    return (Get-FileHash -Path $Path -Algorithm SHA256).Hash -ieq $Expected
}

$version = Get-PinValue 'version'
$sha256 = Get-PinValue 'sha256'
$folder = Join-Path $Downloads "slang-$version"
$compiler = Join-Path $folder 'bin/slangc.exe'
$zip = Join-Path $Downloads "slang-$version.zip"
$partial = "$zip.partial"
$url = "https://github.com/shader-slang/slang/releases/download/v$version/slang-$version-windows-x86_64.zip"

if (Test-Path $compiler) {
    Write-Host "slangc $version is in place at $compiler"
    return
}

New-Item -ItemType Directory -Force $Downloads | Out-Null
if (-not (Test-Path $zip)) {
    Write-Host "downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $partial
    if (-not (Test-ZipHash $partial $sha256)) {
        throw "The download '$partial' does not have the SHA-256 that '$PinFile' pins."
    }
    Move-Item -Path $partial -Destination $zip
}
if (-not (Test-ZipHash $zip $sha256)) {
    throw "'$zip' does not have the SHA-256 that '$PinFile' pins."
}
Expand-Archive -Path $zip -DestinationPath $folder
if (-not (Test-Path $compiler)) { throw "The release zip holds no bin/slangc.exe." }
Write-Host "slangc $version unpacked to $folder"
