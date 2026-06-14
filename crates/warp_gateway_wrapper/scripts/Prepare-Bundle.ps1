param(
    [Parameter(Mandatory = $true)]
    [string]$WrapperExe,

    [Parameter(Mandatory = $true)]
    [string]$GatewayExe,

    [Parameter(Mandatory = $true)]
    [string]$OutputDir,

    [ValidateSet("stable", "preview", "dev", "local", "oss", "integration")]
    [string]$Channel = "stable",

    [string]$ConfigName = "warp-gateway-wrapper.toml",

    [string]$GatewayFileName = "managed-gateway.exe",

    [string]$WarpBinaryPath,

    [string]$WarpBinaryFileName = "warp.exe",

    [string]$ReportPath,

    [switch]$Force
)

$ErrorActionPreference = "Stop"

$resolvedWrapperExe = (Resolve-Path -LiteralPath $WrapperExe).Path
$resolvedGatewayExe = (Resolve-Path -LiteralPath $GatewayExe).Path
$resolvedWarpBinaryPath = $null
if ($WarpBinaryPath) {
    $resolvedWarpBinaryPath = (Resolve-Path -LiteralPath $WarpBinaryPath).Path
}
$bundleDir = [System.IO.Path]::GetFullPath($OutputDir)

New-Item -ItemType Directory -Path $bundleDir -Force | Out-Null

$wrapperDestination = Join-Path $bundleDir ([System.IO.Path]::GetFileName($resolvedWrapperExe))
$gatewayDestination = Join-Path $bundleDir $GatewayFileName
$warpBinaryDestination = $null
if ($resolvedWarpBinaryPath) {
    $warpBinaryDestination = Join-Path $bundleDir $WarpBinaryFileName
}
$configDestination = Join-Path $bundleDir $ConfigName

foreach ($destination in @($wrapperDestination, $gatewayDestination, $warpBinaryDestination, $configDestination)) {
    if (-not $destination) {
        continue
    }
    if ((Test-Path -LiteralPath $destination) -and -not $Force) {
        throw "Bundle output already exists at $destination. Re-run with -Force to overwrite."
    }
}

Copy-Item -LiteralPath $resolvedWrapperExe -Destination $wrapperDestination -Force
Copy-Item -LiteralPath $resolvedGatewayExe -Destination $gatewayDestination -Force
if ($resolvedWarpBinaryPath) {
    Copy-Item -LiteralPath $resolvedWarpBinaryPath -Destination $warpBinaryDestination -Force
}

$initArgs = @("--config", $configDestination, "init-config", "--channel", $Channel)
if ($Force) {
    $initArgs += "--force"
}
& $wrapperDestination @initArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if ($warpBinaryDestination) {
    $configContents = Get-Content -LiteralPath $configDestination -Raw
    $warpBinaryConfigPath = ".\" + [System.IO.Path]::GetFileName($warpBinaryDestination)
    $configContents = $configContents -replace "channel = ""$Channel""", "channel = ""$Channel""`r`nbinary = ""$warpBinaryConfigPath"""
    Set-Content -LiteralPath $configDestination -Value $configContents -Encoding UTF8
}

if (-not $ReportPath) {
    $ReportPath = Join-Path $bundleDir "bundle-check.json"
}

$bundleCheckScript = Join-Path $PSScriptRoot "Invoke-BundleCheck.ps1"
& $bundleCheckScript `
    -WrapperExe $wrapperDestination `
    -ConfigPath $configDestination `
    -ReportPath $ReportPath
exit $LASTEXITCODE
