[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$RepositoryRoot = Split-Path -Parent $PSScriptRoot
$XrayVersion = "v26.3.27"
$XraySha256 = "d004c39288ce9ada487c6f398c7c545f7d749e44bdfdd59dbc9f865afba4e1ad"
$XrayExecutableSha256 = "15c2d007954ac53ba69b80ec91242786b3c0b71d52649165b4ca1d5cc96ef8f1"

$XrayOutput = Join-Path $RepositoryRoot "src-tauri\resources\xray"
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

New-Item -ItemType Directory -Force -Path $TemporaryRoot, $XrayOutput | Out-Null

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

    Write-Host "Prepared pinned Xray assets."
} finally {
    if (Test-Path -LiteralPath $TemporaryRoot) {
        Remove-Item -LiteralPath $TemporaryRoot -Recurse -Force
    }
}
