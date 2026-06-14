param(
    [string]$CargoExe = "cargo",

    [string]$Profile = "release",

    [string]$Target,

    [string]$OutputDir
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$crateDir = Split-Path -Parent $scriptDir
$workspaceRoot = Split-Path -Parent (Split-Path -Parent $crateDir)

$cargoArgs = @("build", "-p", "warp_gateway_wrapper", "--bin", "warp-gateway-wrapper")
if ($Profile -eq "release") {
    $cargoArgs += "--release"
}
else {
    $cargoArgs += @("--profile", $Profile)
}
if ($Target) {
    $cargoArgs += @("--target", $Target)
}

Push-Location $workspaceRoot
try {
    & $CargoExe @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}
finally {
    Pop-Location
}

$targetRoot = Join-Path $workspaceRoot "target"
if ($Target) {
    $targetRoot = Join-Path $targetRoot $Target
}
$builtExe = Join-Path (Join-Path $targetRoot $Profile) "warp-gateway-wrapper.exe"

if (-not (Test-Path -LiteralPath $builtExe)) {
    throw "Built wrapper executable was not found at $builtExe"
}

if ($OutputDir) {
    $resolvedOutputDir = [System.IO.Path]::GetFullPath($OutputDir)
    New-Item -ItemType Directory -Path $resolvedOutputDir -Force | Out-Null
    $destination = Join-Path $resolvedOutputDir "warp-gateway-wrapper.exe"
    Copy-Item -LiteralPath $builtExe -Destination $destination -Force
    Write-Output $destination
}
else {
    Write-Output $builtExe
}
