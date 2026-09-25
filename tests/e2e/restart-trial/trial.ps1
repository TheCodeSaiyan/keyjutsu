# Runs inside Windows Sandbox only. Phase 1, a real restart of the Sandbox,
# then phase 2. Everything is logged to work\trial.log, which the host reads.
$kj = 'C:\kj'
$w = "$kj\work"
$exe = "$kj\bin\boundary_trial.exe"
New-Item -ItemType Directory -Force $w | Out-Null
function log($m) { Add-Content -LiteralPath "$w\trial.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }

# Refuse to run anywhere but the Sandbox's own account.
if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }

if (Test-Path "$w\phase2.started") { log "launcher ran again after phase 2 started; nothing to do"; exit }
if (Test-Path "$w\phase1.done") {
    New-Item -ItemType File "$w\phase2.started" | Out-Null
    log "after the restart: running phase 2"
    & $exe phase2 $w *>> "$w\trial.log"
    log "phase 2 exited $LASTEXITCODE"
    New-Item -ItemType File "$w\all.done" | Out-Null
    exit
}
log "first start: running phase 1"
& $exe phase1 $w *>> "$w\trial.log"
if ($LASTEXITCODE -ne 0) { log "phase 1 failed ($LASTEXITCODE); not restarting"; New-Item -ItemType File "$w\all.done" | Out-Null; exit }
New-Item -ItemType File "$w\phase1.done" | Out-Null
reg.exe add 'HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce' /v KeyJutsuTrial /t REG_SZ /d "powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File $kj\trial.ps1" /f | Out-Null
log "restarting the Sandbox in 5 seconds"
shutdown.exe /r /t 5 /c "KeyJutsu boundary trial"
