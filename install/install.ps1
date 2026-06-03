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

function Normalize-PathEntry {
    param([string]$PathEntry)

    try {
        return [System.IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($PathEntry)).TrimEnd('\')
    }
    catch {
        return $PathEntry.TrimEnd('\')
    }
}

function Add-ToUserPath {
    param([string]$Dir)

    $fullDir = Normalize-PathEntry -PathEntry $Dir
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if ($userPath) {
        $entries = $userPath -split ";" | Where-Object { $_ -ne "" }
    }

    $alreadyInUserPath = $entries | Where-Object {
        (Normalize-PathEntry -PathEntry $_).Equals($fullDir, [System.StringComparison]::OrdinalIgnoreCase)
    } | Select-Object -First 1

    if (-not $alreadyInUserPath) {
        $nextPath = if ($userPath) { "$userPath;$fullDir" } else { $fullDir }
        [Environment]::SetEnvironmentVariable("Path", $nextPath, "User")
        Write-Host "Added to user PATH: $fullDir"
    }

    $currentEntries = $env:Path -split ";" | Where-Object { $_ -ne "" }
    $alreadyInCurrentPath = $currentEntries | Where-Object {
        (Normalize-PathEntry -PathEntry $_).Equals($fullDir, [System.StringComparison]::OrdinalIgnoreCase)
    } | Select-Object -First 1

    if (-not $alreadyInCurrentPath) {
        $env:Path = "$env:Path;$fullDir"
    }
}

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

function Get-Sha256FileHash {
    param([string]$Path)

    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        return (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLowerInvariant()
    }

    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            $hashBytes = $sha256.ComputeHash($stream)
            return ([System.BitConverter]::ToString($hashBytes) -replace "-", "").ToLowerInvariant()
        }
        finally {
            $sha256.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Copy-InstalledExeWithRetry {
    param(
        [string]$Source,
        [string]$Destination,
        [int]$Attempts = 80,
        [int]$DelayMs = 500
    )

    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        try {
            Copy-Item -Path $Source -Destination $Destination -Force
            return
        }
        catch [System.IO.IOException] {
            if ($attempt -eq $Attempts) {
                throw "failed to update $Destination because it is still in use. Close running RepoLens processes and run `repolens self-update` again."
            }
            Start-Sleep -Milliseconds $DelayMs
        }
    }
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
    $actual = Get-Sha256FileHash -Path $archivePath
    if ($expected -ne $actual) {
        throw "checksum mismatch"
    }

    Expand-Archive -Path $archivePath -DestinationPath $tmp -Force
    $exe = Get-ChildItem -Path $tmp -Filter repolens.exe -Recurse | Select-Object -First 1
    if (-not $exe) {
        throw "repolens.exe not found in archive"
    }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-InstalledExeWithRetry -Source $exe.FullName -Destination $installedExe
    Add-ToUserPath -Dir $InstallDir

    if ($Action -eq "update") {
        Write-Host "Updated repolens in $InstallDir"
    }
    else {
        Write-Host "Installed repolens to $InstallDir"
    }
    Write-Host ""
    Write-Host "RepoLens is installed globally."
    Write-Host "To activate it for a project, open that project folder and run:"
    Write-Host "  repolens init . --target codex"

    if ($Init) {
        Write-Host "Configuring MCP target '$InitTarget' for repo root:"
        Write-Host "  $InitRoot"
        & $installedExe init --target $InitTarget $InitRoot
    }
}
finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
