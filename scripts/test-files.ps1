param(
    [ValidateSet("all", "stages", "additional")]
    [string]$Suite = "all",
    [string]$Filter = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    $targets = switch ($Suite) {
        "stages" { @("stages") }
        "additional" { @("additional") }
        default { @("stages", "additional") }
    }

    $testExitCode = 0
    foreach ($target in $targets) {
        $cargoArgs = @("test", "--manifest-path", "src-tauri/Cargo.toml", "--test", $target)
        if ($Filter) { $cargoArgs += $Filter }
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            $testExitCode = $LASTEXITCODE
            break
        }
    }
} finally {
    Pop-Location
}
if ($testExitCode -ne 0) { exit $testExitCode }
