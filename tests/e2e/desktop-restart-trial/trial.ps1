# Runs inside Windows Sandbox only. Installs KeyJutsu, approves and runs the
# first phase of a plan in the app, restarts the Sandbox for real, then
# continues the plan from the app's offer and checks it finished. Logs to
# work\trial.log and takes a screenshot at each stage.
$kj = 'C:\kj'
$w = "$kj\work"
New-Item -ItemType Directory -Force $w | Out-Null
function log($m) { Add-Content -LiteralPath "$w\trial.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }

if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }
if (Test-Path "$w\phase2.started") { log "launcher ran again after phase 2 started; nothing to do"; exit }

$app = "$env:ProgramFiles\KeyJutsu\keyjutsu-desktop.exe"

# Each action is tried a few times: a new window is not always allowed the
# foreground at once, and the driver refuses until it is.
function d($action, $arg = '', $shot = '') {
    for ($try = 1; $try -le 5; $try++) {
        # Windows PowerShell drops an empty argument: pass only what has one.
        $call = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$kj\drive-desktop.ps1", '-Action', $action)
        if ($arg) { $call += @('-Arg', $arg) }
        if ($shot) { $call += @('-Out', "$w\$shot.png") }
        powershell.exe @call 2>&1 | ForEach-Object { log "  $_" }
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 2
    }
    log "gave up on $action $arg"
}
function launch {
    Get-Process keyjutsu-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Process -FilePath $app -ArgumentList '--cwd', 'C:\Users\Public' | Out-Null
    Start-Sleep -Seconds 15
}
# The plan screen, in Direct mode (no keys needed), armed.
function arm {
    d click 'Terminal'
    Start-Sleep -Seconds 8
    d click 'Direct'
    d click 'Plan'
    Start-Sleep -Seconds 2
}

if (Test-Path "$w\phase1.done") {
    New-Item -ItemType File "$w\phase2.started" | Out-Null
    log "after the restart: opening KeyJutsu"
    launch
    d capture '' 'waiting-banner'
    d click 'Continue it'
    Start-Sleep -Seconds 3
    arm
    d click 'Arm KeyJutsu'
    # The checks on this side of the restart, then the question.
    Start-Sleep -Seconds 15
    d capture '' 'resume-dialog'
    d keys 'RESUME'
    d click 'Continue the plan'
    Start-Sleep -Seconds 25
    d capture '' 'resumed'
    $result = (Get-Content -Raw -LiteralPath "$w\result.txt" -ErrorAction SilentlyContinue)
    if ($result -and $result.Trim() -eq 'finished-after-restart') {
        log 'PASS: the app offered the waiting plan after the restart, asked, and finished it'
    } else {
        log "FAIL: result.txt is '$result'"
    }
    New-Item -ItemType File "$w\all.done" | Out-Null
    exit
}

log 'first start: installing'
$installer = Get-ChildItem "$kj\installer\*.exe" | Select-Object -First 1
$p = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
log "installer exited $($p.ExitCode)"
if (-not (Test-Path $app)) { log 'FAIL: not installed'; New-Item -ItemType File "$w\all.done" | Out-Null; exit }

# Validated on this machine by the CLI, as the docs capture does, so the app
# opens it ready to approve.
$cli = "$env:ProgramFiles\KeyJutsu\keyjutsu.exe"
& $cli plan approve "$kj\plan.json" --out "$w\approved.json" 2>&1 | ForEach-Object { log "  $_" }
(Get-Content -Raw "$w\approved.json" | ConvertFrom-Json).plan | ConvertTo-Json -Depth 50 |
    Set-Content -LiteralPath "$w\validated.json"
$env:KEYJUTSU_OPEN_PLAN = "$w\validated.json"
launch
Remove-Item Env:KEYJUTSU_OPEN_PLAN
arm
d click 'Approve plan'
Start-Sleep -Seconds 6
d click 'Arm KeyJutsu'
Start-Sleep -Seconds 25
d capture '' 'stopped-at-boundary'
if (-not (Test-Path "$w\marker.txt") -or (Test-Path "$w\result.txt")) {
    log 'FAIL: phase 1 did not stop at the boundary as it should'
    New-Item -ItemType File "$w\all.done" | Out-Null
    exit
}
log 'phase 1 ran and stopped at the restart; restarting the Sandbox'
Get-Process keyjutsu-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
New-Item -ItemType File "$w\phase1.done" | Out-Null
reg.exe add 'HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce' /v KeyJutsuTrial /t REG_SZ /d "powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File $kj\trial.ps1" /f | Out-Null
shutdown.exe /r /t 5 /c "KeyJutsu desktop restart trial"
