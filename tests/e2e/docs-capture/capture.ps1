# Runs inside Windows Sandbox only: the documentation's example output and
# screenshots, taken on a clean Windows 11 rather than anyone's own machine.
# Installs KeyJutsu, gives it stand-in agents (answering --version with the
# versions the adapters were checked against), runs each documented command
# from a neutral folder, and drives the app for the screenshots. Everything
# lands in C:\kj\out.
$kj = 'C:\kj'
$out = "$kj\out"
New-Item -ItemType Directory -Force $out, "$kj\work" | Out-Null
function log($m) { Add-Content -LiteralPath "$out\capture.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }

if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }
if (Test-Path "$out\started") { exit }
New-Item -ItemType File "$out\started" | Out-Null

# Install, as the installing guide does it.
$installer = Get-ChildItem "$kj\installer\*.exe" | Select-Object -First 1
$p = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
log "installer exited $($p.ExitCode)"
$app = "$env:ProgramFiles\KeyJutsu\keyjutsu-desktop.exe"

# Stand-in agents: no real agent, no account, nobody's versions.
New-Item -ItemType Directory -Force "$kj\agents" | Out-Null
Set-Content -LiteralPath "$kj\agents\codex.cmd" -Value '@echo codex-cli 0.154.0'
Set-Content -LiteralPath "$kj\agents\claude.cmd" -Value '@echo 2.1.282 (Claude Code)'
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
    [Environment]::GetEnvironmentVariable('Path', 'User') + ";$kj\agents"

# The documented commands, run where a reader would: a folder of their own.
$docs = 'C:\Users\Public\Documents\keyjutsu-examples'
New-Item -ItemType Directory -Force $docs | Out-Null
Copy-Item "$kj\examples\*.json" $docs
Set-Location $docs
New-Item -ItemType Directory -Force 'C:\Users\Public\BuildCache' | Out-Null
function run($name, [string[]] $cmd) {
    $text = (& keyjutsu @cmd 2>&1 | Out-String)
    Set-Content -LiteralPath "$out\$name.txt" -Value ("> keyjutsu " + ($cmd -join ' ') + "`n" + $text + "[exit $LASTEXITCODE]")
    log "$name exit $LASTEXITCODE"
}
run 'doctor' @('doctor')
run 'agents' @('agents')
run 'plan-check' @('plan', 'check', 'check-a-service.json')
run 'plan-hash' @('plan', 'hash', 'check-a-service.json')
run 'plan-validate' @('plan', 'validate', 'check-a-service.json')
run 'plan-approve' @('plan', 'approve', 'check-a-service.json', '--out', 'check-a-service.approved.json')
run 'plan-verify' @('plan', 'verify', 'check-a-service.approved.json', '--environment')
run 'plan-approve-critical' @('plan', 'approve', 'clear-build-cache.json', '--out', 'clear-build-cache.approved.json')
run 'plan-approve-critical-confirmed' @('plan', 'approve', 'clear-build-cache.json', '--out', 'clear-build-cache.approved.json',
    '--confirm', 'clear-cache=DELETE THE OLD BUILD CACHE')
run 'plan-propose' @('plan', 'propose', 'Find out why the Print Spooler keeps stopping', '--agent', 'claude', '--out', 'spooler.json')
run 'setup-status' @('setup', 'path', 'status')
run 'diagnostics' @('diagnostics', 'preview')

# The plans with this machine's validation recorded, for the Plan screen.
foreach ($pair in @(@('check-a-service', 'validated'), @('clear-build-cache', 'critical-validated'))) {
    $snap = Get-Content -Raw "$docs\$($pair[0]).approved.json" | ConvertFrom-Json
    $snap.plan | ConvertTo-Json -Depth 50 | Set-Content -LiteralPath "$kj\work\$($pair[1]).json"
}

# The app. Each action is tried a few times: a new window is not always
# allowed the foreground at once, and the driver refuses until it is.
function d($action, $arg = '', $shot = '') {
    for ($try = 1; $try -le 5; $try++) {
        # Windows PowerShell drops an empty argument, and the script then
        # refuses a parameter with no value: pass only what has one.
        $call = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$kj\drive-desktop.ps1", '-Action', $action)
        if ($arg) { $call += @('-Arg', $arg) }
        if ($shot) { $call += @('-Out', "$out\$shot.png") }
        powershell.exe @call 2>&1 | ForEach-Object { log "  $_" }
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 2
    }
    log "gave up on $action $arg"
}
function launch([string] $plan) {
    Get-Process keyjutsu-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2
    if ($plan) { $env:KEYJUTSU_OPEN_PLAN = $plan } else { Remove-Item Env:KEYJUTSU_OPEN_PLAN -ErrorAction SilentlyContinue }
    Start-Process -FilePath $app -ArgumentList '--cwd', 'C:\Users\Public' | Out-Null
    Start-Sleep -Seconds 15
}

launch ''
d capture '' 'first-run'
d click 'Try the safe demo'
Start-Sleep -Seconds 4
d capture '' 'new-task'
d click 'Terminal'
Start-Sleep -Seconds 8
d click 'Profile'
Start-Sleep -Milliseconds 800
d chord '{DOWN}'
d chord '{ENTER}'
Start-Sleep -Seconds 1
d click 'New terminal'
Start-Sleep -Seconds 10
d capture '' 'terminal'
d click 'Arm KeyJutsu'
Start-Sleep -Seconds 2
d keys 'asdfjklqwertyuiopasdfjklzxcvbnm'
Start-Sleep -Seconds 2
d chord '^+k'                     # operator controls
Start-Sleep -Seconds 2
d capture '' 'performance'
d click 'Disarm'                  # from the operator controls
Start-Sleep -Seconds 2

# The diagnostic bundle: previewed, then saved, and the saved file kept to
# compare with what the preview showed.
d click 'Preview bundle'
Start-Sleep -Seconds 25
d capture '' 'diagnostics'
d click 'Save bundle'
Start-Sleep -Seconds 3
Get-ChildItem "$env:LOCALAPPDATA\KeyJutsu\diagnostics\*.txt" -ErrorAction SilentlyContinue |
    ForEach-Object { Copy-Item $_.FullName "$out\diagnostics-saved.txt"; log "saved bundle $($_.Name)" }

launch "$kj\work\validated.json"
d capture '' 'plan-workspace'

# A run in the app, recorded, made a Technique and used again: the end of
# the whole journey. Direct mode, so no keys are needed.
launch "$kj\work\validated.json"
d click 'Terminal'
Start-Sleep -Seconds 6
d click 'Direct'
d click 'Plan'
Start-Sleep -Seconds 2
d click 'Approve plan'
Start-Sleep -Seconds 6
d click 'Arm KeyJutsu'
Start-Sleep -Seconds 30
d capture '' 'run-complete'
d click 'History'
Start-Sleep -Seconds 3
d click 'Check that Windows Management Instrumentation'
Start-Sleep -Seconds 3
d click 'Parameters, one per line'
d keys 'service_name = Winmgmt'
Start-Sleep -Seconds 1
d capture '' 'history'
d click 'Make a Technique'
Start-Sleep -Seconds 3
d click 'Check that Windows Management Instrumentation'
Start-Sleep -Seconds 2
d capture '' 'techniques'
d click 'Make a draft plan'
Start-Sleep -Seconds 5
d capture '' 'technique-draft'

launch "$kj\work\critical-validated.json"
d click 'Approve plan'
Start-Sleep -Seconds 4
d capture '' 'critical-step'
Get-Process keyjutsu-desktop -ErrorAction SilentlyContinue | Stop-Process -Force

log 'done'
New-Item -ItemType File "$out\all.done" | Out-Null
