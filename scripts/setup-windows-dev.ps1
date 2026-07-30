[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Install-WingetPackage {
    param(
        [Parameter(Mandatory = $true)][string]$Id,
        [string]$Override = ""
    )

    $Arguments = @("install", "--id", $Id, "--exact", "--accept-package-agreements", "--accept-source-agreements")
    if ($Override) {
        $Arguments += @("--override", $Override)
    }
    & winget @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "winget failed while installing $Id"
    }
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    throw "winget is required. Install or update Windows App Installer first."
}

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Install-WingetPackage -Id "OpenJS.NodeJS.LTS"
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Install-WingetPackage -Id "Rustlang.Rustup"
    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
}

$VsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$HasBuildTools = Test-Path -LiteralPath $VsWhere
if ($HasBuildTools) {
    $Installation = & $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $HasBuildTools = -not [string]::IsNullOrWhiteSpace($Installation)
}
if (-not $HasBuildTools) {
    Install-WingetPackage -Id "Microsoft.VisualStudio.2022.BuildTools" -Override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
}

Write-Host "Windows development prerequisites are installed."
Write-Host "Open a new terminal, then run scripts\run-windows.cmd."
