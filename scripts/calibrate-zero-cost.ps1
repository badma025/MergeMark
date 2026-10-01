param(
    [string]$Root = ".",
    [string]$OutDir = "output/zero-cost-orchestration/c-calibration",
    [string]$Stamp = ""
)
# DEPRECATED SHIM. The previous version of this script only ran
# `--inspect-source` and then read whatever certification artifact already
# existed in a fixed directory, so it could report stale or fabricated results.
# Real calibration is now performed by the fresh-runner below, which launches
# the actual offline ingestion into a unique per-run/per-paper directory,
# checks the child exit code and artifact freshness, and validates the
# zero-attempt policy. This shim only forwards arguments for compatibility.
$ErrorActionPreference = "Stop"
Set-Location $Root
$args = @("scripts/run-zero-cost-calibration.mjs", "--out", $OutDir)
if ($Stamp -ne "") { $args += @("--stamp", $Stamp) }
& node @args
exit $LASTEXITCODE
