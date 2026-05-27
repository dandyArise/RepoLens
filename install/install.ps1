param(
    [ValidateSet("install", "update", "uninstall", "enable", "disable", "status")]
    [string]$Action = "install",
    [string]$Version = "latest",
    [string]$InstallDir = "$env:USERPROFILE\bin",
    [string]$Repo = "dandyArise/RepoLens",
    [switch]$Init,
    [ValidateSet("all", "codex", "claude", "cursor")]
    [string]$InitTarget = "all",
    [string]$InitRoot = (Get-Location).Path
)

$ErrorActionPreference = "Stop"

$installedExe = Join-Path $InstallDir "repolens.exe"

if ($Action -eq "uninstall") {
    if (Test-Path -LiteralPath $installedExe) {
        Remove-Item -LiteralPath $installedExe -Force
        Write-Host "Removed $installedExe"
    }
    else {
        Write-Host "repolens is not installed at $installedExe"
    }
    exit 0
}

if ($Action -in @("enable", "disable", "status")) {
    if (-not (Test-Path -LiteralPath $installedExe)) {
        throw "repolens is not installed at $installedExe"
    }

    if ($Action -eq "enable") {
        & $installedExe enable --target $InitTarget $InitRoot
    }
    elseif ($Action -eq "disable") {
        & $installedExe disable --target $InitTarget
    }
    else {
        & $installedExe mcp-status --target $InitTarget
    }
    exit $LASTEXITCODE
}

function Get-Release {
    param([string]$Repo, [string]$Version)

    $headers = @{ "User-Agent" = "repolens-installer" }
    if ($Version -eq "latest") {
        return Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases/latest"
    }

    $tag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
    return Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases/tags/$tag"
}

function Select-Asset {
    param($Release, [string]$Name)

    $asset = $Release.assets | Where-Object { $_.name -eq $Name } | Select-Object -First 1
    if (-not $asset) {
        throw "release asset not found: $Name"
    }
    return $asset
}

$assetName = "repolens-windows-x86_64.zip"
$release = Get-Release -Repo $Repo -Version $Version
$archiveAsset = Select-Asset -Release $release -Name $assetName
$checksumAsset = Select-Asset -Release $release -Name "$assetName.sha256"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("repolens-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
    $archivePath = Join-Path $tmp $assetName
    $checksumPath = Join-Path $tmp "$assetName.sha256"

    Invoke-WebRequest -UseBasicParsing -Uri $archiveAsset.browser_download_url -OutFile $archivePath
    Invoke-WebRequest -UseBasicParsing -Uri $checksumAsset.browser_download_url -OutFile $checksumPath

    $expected = (Get-Content $checksumPath -Raw).Trim().Split(" ")[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        throw "checksum mismatch"
    }

    Expand-Archive -Path $archivePath -DestinationPath $tmp -Force
    $exe = Get-ChildItem -Path $tmp -Filter repolens.exe -Recurse | Select-Object -First 1
    if (-not $exe) {
        throw "repolens.exe not found in archive"
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -Path $exe.FullName -Destination $installedExe -Force

    if ($Action -eq "update") {
        Write-Host "Updated repolens in $InstallDir"
    }
    else {
        Write-Host "Installed repolens to $InstallDir"
    }
    Write-Host "Add this directory to PATH if needed:"
    Write-Host "  $InstallDir"

    if ($Init) {
        Write-Host "Configuring MCP target '$InitTarget' for repo root:"
        Write-Host "  $InitRoot"
        & $installedExe init --target $InitTarget $InitRoot
    }
}
finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
