[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$XrayVersion = "v26.3.27"
$XraySha256 = "d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad"
$XrayExecutableSha256 = "15c2d007954ac53ba69b80ec91242786b3c0b71d52649165b4ca1d5cc96ef8f1"
$CodexCommit = "6219b7c40fc9c702c0aef9964e72b492558f60e4"
$CodexInstallerSha256 = "391f247de2c70c7e99041979ec02dae7e76be27ac9cfc1dfe7c1eb21d48d8b97"
$CodexLicenseSha256 = "d17f227e4df5da1600391338865ce0f3055211760a36688f816941d58232d8dc"
$CodexNoticeSha256 = "9d71575ecfd9a843fc1677b0efb08053c6ba9fd686a0de1a6f5382fd3c220915"

$XrayOutput = Join-Path $RepositoryRoot "src-tauri\resources\xray"
$CodexOutput = Join-Path $RepositoryRoot "src-tauri\resources\codex"
$TemporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("codex-go-assets-" + [Guid]::NewGuid().ToString("N"))

function Assert-Sha256 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected
    )
    $Actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "SHA-256 mismatch for $(Split-Path -Leaf $Path). Expected $Expected, got $Actual."
    }
}

function Save-VerifiedDownload {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][string]$Expected,
        [int]$TimeoutSeconds = 180
    )

    $StagingPath = "$Destination.download-$([Guid]::NewGuid().ToString('N'))"
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $StagingPath -TimeoutSec $TimeoutSeconds
        Assert-Sha256 -Path $StagingPath -Expected $Expected
        Move-Item -LiteralPath $StagingPath -Destination $Destination -Force
    } finally {
        if (Test-Path -LiteralPath $StagingPath) {
            Remove-Item -LiteralPath $StagingPath -Force
        }
    }
}

New-Item -ItemType Directory -Force -Path $TemporaryRoot, $XrayOutput, $CodexOutput | Out-Null

try {
    $XrayArchive = Join-Path $TemporaryRoot "Xray-windows-64.zip"
    $XrayExpanded = Join-Path $TemporaryRoot "xray"
    $XrayUrl = "https://github.com/XTLS/Xray-core/releases/download/$XrayVersion/Xray-windows-64.zip"
    Invoke-WebRequest -UseBasicParsing -Uri $XrayUrl -OutFile $XrayArchive
    Assert-Sha256 -Path $XrayArchive -Expected $XraySha256
    Expand-Archive -LiteralPath $XrayArchive -DestinationPath $XrayExpanded -Force
    $XrayExecutable = Join-Path $XrayExpanded "xray.exe"
    Assert-Sha256 -Path $XrayExecutable -Expected $XrayExecutableSha256
    Copy-Item -LiteralPath $XrayExecutable -Destination (Join-Path $XrayOutput "xray.exe") -Force
    Copy-Item -LiteralPath (Join-Path $XrayExpanded "LICENSE") -Destination (Join-Path $XrayOutput "LICENSE-XRAY.txt") -Force

    $CodexInstaller = Join-Path $CodexOutput "install.ps1"
    $CodexInstallerUrl = "https://raw.githubusercontent.com/openai/codex/$CodexCommit/scripts/install/install.ps1"
    Save-VerifiedDownload -Uri $CodexInstallerUrl -Destination $CodexInstaller -Expected $CodexInstallerSha256

    $CodexLicense = Join-Path $CodexOutput "LICENSE-APACHE-2.0.txt"
    $CodexLicenseUrl = "https://raw.githubusercontent.com/openai/codex/$CodexCommit/LICENSE"
    Save-VerifiedDownload -Uri $CodexLicenseUrl -Destination $CodexLicense -Expected $CodexLicenseSha256

    $CodexNotice = Join-Path $CodexOutput "NOTICE-CODEX.txt"
    $CodexNoticeUrl = "https://raw.githubusercontent.com/openai/codex/$CodexCommit/NOTICE"
    Save-VerifiedDownload -Uri $CodexNoticeUrl -Destination $CodexNotice -Expected $CodexNoticeSha256

    Write-Host "Prepared pinned Xray and Codex installer assets."
} finally {
    if (Test-Path -LiteralPath $TemporaryRoot) {
        Remove-Item -LiteralPath $TemporaryRoot -Recurse -Force
    }
}
