# Spike #11: cold start, measured. Before each launch the file cache (standby list) is emptied,
# so jerry.exe, the WebView2 runtime and the listed folder come from disk as after a reboot, and
# launches are 25 s apart so the discrete GPU is asleep as well. 5 launches of the optimized
# startup alternate with 3 of the baseline (listing started by the UI, no window background).
#
#   powershell -ExecutionPolicy Bypass -File scripts\spike-11-cold.ps1
#
# One UAC prompt (the cache purge needs administrator rights; Jerry itself runs as you).
# Takes about 5 minutes. Don't use the mouse or keyboard meanwhile: each run waits for an idle
# machine. Results: %TEMP%\jerry-spike-11\out\cold.jsonl (cold-reboot.jsonl with -AfterReboot)
# and the table printed at the end.
#
# -AfterReboot: right after a reboot, one launch without purging (the real thing).

param(
  [switch]$AfterReboot,
  [int]$Runs = 5,
  [string]$Jerry = "$env:TEMP\jerry-spike-11\jerry-v2.exe",
  [string]$Probe = "$env:TEMP\jerry-spike-11\startup_probe.exe"
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$work = "$env:TEMP\jerry-spike-11"
$purgeDir = "$work\purge"
$out = if ($AfterReboot) { "$work\out\cold-reboot.jsonl" } else { "$work\out\cold.jsonl" }
New-Item -ItemType Directory -Force "$work\out", $purgeDir | Out-Null
foreach ($file in @($Jerry, $Probe)) {
  if (-not (Test-Path $file)) { throw "Missing $file (built by the spike session)." }
}
$dgpu = (Get-PnpDevice -Class Display -PresentOnly | Where-Object { $_.InstanceId -like "PCI\VEN_10DE*" } |
  Select-Object -First 1).InstanceId
$probeArgs = @($Jerry, $out, "--warmup", "0", "--idle", "2", "--cpu", "30")
if ($dgpu) { $probeArgs += @("--dgpu", $dgpu) }

if ($AfterReboot) {
  & $Probe @probeArgs --runs 1 --label "C cold after reboot, optimized"
} else {
  Remove-Item "$purgeDir\*", $out -Force -ErrorAction SilentlyContinue
  $helper = Start-Process powershell -Verb RunAs -PassThru -WindowStyle Minimized -ArgumentList @(
    "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSScriptRoot\spike-11-purge.ps1`"",
    "-Dir", "`"$purgeDir`"")
  try {
    for ($i = 1; $i -le $Runs; $i++) {
      # 25 s apart: the discrete GPU falls asleep in between, as before a real launch.
      Start-Sleep -Seconds 25
      & $Probe @probeArgs --runs 1 --purge $purgeDir --label "C cold, optimized"
      if ($i -le 3) {
        Start-Sleep -Seconds 25
        & $Probe @probeArgs --runs 1 --purge $purgeDir --label "C cold, baseline" `
          --env JERRY_SPIKE_LISTING=ui --env JERRY_SPIKE_BG=0
      }
    }
  } finally {
    Set-Content -Path "$purgeDir\stop" -Value "stop" -Encoding ascii
    if (-not $helper.HasExited) { $helper.WaitForExit(5000) | Out-Null }
  }
}

node "$repo\scripts\spike-11-summary.mjs" $out
Write-Host "`nResults: $out"
