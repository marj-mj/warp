param(
    [Parameter(Mandatory = $true)]
    [string]$WrapperExe,

    [Parameter(Mandatory = $true)]
    [string]$ConfigPath,

    [string]$ReportPath
)

$ErrorActionPreference = "Stop"

$resolvedWrapperExe = (Resolve-Path -LiteralPath $WrapperExe).Path
$resolvedConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path

$output = & $resolvedWrapperExe --config $resolvedConfigPath bundle-check --json 2>&1
$exitCode = $LASTEXITCODE
$outputText = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine

if ($ReportPath) {
    $reportDirectory = Split-Path -Parent $ReportPath
    if ($reportDirectory) {
        New-Item -ItemType Directory -Path $reportDirectory -Force | Out-Null
    }
    Set-Content -LiteralPath $ReportPath -Value $outputText -Encoding UTF8
}

$outputText
exit $exitCode
