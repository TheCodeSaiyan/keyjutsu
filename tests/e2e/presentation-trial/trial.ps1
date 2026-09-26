# Runs inside Windows Sandbox only. "Keep the illusion" in the desktop app,
# end to end: a critical step whose approval has aged past its hour is asked
# for without covering the terminal, the question opens as a corner card on
# Ctrl+Shift+K, and after the run the stage is held, with keys going nowhere,
# until Ctrl+Shift+K lets go. The Sandbox's clock is moved on two hours to age
# the approval, which is why this runs nowhere else.
$kj = 'C:\kj'
$w = "$kj\work"
New-Item -ItemType Directory -Force $w | Out-Null
function log($m) { Add-Content -LiteralPath "$w\trial.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }
$script:failed = $false
function check($what, $ok) { if ($ok) { log "ok   $what" } else { log "FAIL $what"; $script:failed = $true } }

if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }
if (Test-Path "$w\started") { exit }
New-Item -ItemType File "$w\started" | Out-Null

$app = "$env:ProgramFiles\KeyJutsu\keyjutsu-desktop.exe"
function d($action, $arg = '', $shot = '') {
    for ($try = 1; $try -le 5; $try++) {
        $call = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$kj\drive-desktop.ps1", '-Action', $action)
        if ($arg) { $call += @('-Arg', $arg) }
        if ($shot) { $call += @('-Out', "$w\$shot.png") }
        powershell.exe @call 2>&1 | ForEach-Object { log "  $_" }
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 2
    }
    log "gave up on $action $arg"
}
# Whether an element is on screen, asked once.
function has($name) {
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$kj\drive-desktop.ps1" -Action has -Arg $name 2>&1 |
        ForEach-Object { log "  $_" }
    return ($LASTEXITCODE -eq 0)
}

log 'installing'
$installer = Get-ChildItem "$kj\installer\*.exe" | Select-Object -First 1
$p = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
log "installer exited $($p.ExitCode)"

$cache = 'C:\Users\Public\BuildCache'
New-Item -ItemType Directory -Force $cache | Out-Null
Set-Content -LiteralPath "$cache\old.bin" -Value 'old'

# Validated here, as the docs capture does, so the app opens it ready.
$cli = "$env:ProgramFiles\KeyJutsu\keyjutsu.exe"
& $cli plan approve "$kj\clear-build-cache.json" --out "$w\approved.json" --confirm 'clear-cache=DELETE THE OLD BUILD CACHE' 2>&1 |
    ForEach-Object { log "  $_" }
(Get-Content -Raw "$w\approved.json" | ConvertFrom-Json).plan | ConvertTo-Json -Depth 50 |
    Set-Content -LiteralPath "$w\validated.json"
$env:KEYJUTSU_OPEN_PLAN = "$w\validated.json"
Start-Process -FilePath $app -ArgumentList '--cwd', 'C:\Users\Public' | Out-Null
Start-Sleep -Seconds 15

d click 'Terminal'
Start-Sleep -Seconds 8
d click 'Direct'
# "When KeyJutsu needs you": the second choice, Keep the illusion.
d click 'When KeyJutsu needs you'
Start-Sleep -Milliseconds 800
d chord '{DOWN}'
d chord '{ENTER}'
Start-Sleep -Seconds 1
d capture '' 'setting'
d click 'Plan'
Start-Sleep -Seconds 2
d click 'Approve plan'
Start-Sleep -Seconds 3
d keys 'DELETE THE OLD BUILD CACHE'
d click 'Approve critical step'
Start-Sleep -Seconds 6

# Two hours on: the approval is past its hour when the step is reached.
Set-Date (Get-Date).AddHours(2) | Out-Null
log "clock moved on to $(Get-Date -Format o)"
d click 'Arm KeyJutsu'
Start-Sleep -Seconds 15
d capture '' 'waiting'
check 'the question does not cover the terminal' (-not (has 'Run critical step'))
check 'the step has not run while waiting' (Test-Path "$cache\old.bin")

d chord '^+k'
Start-Sleep -Seconds 2
d capture '' 'card'
check 'Ctrl+Shift+K opens the question' (has 'Run critical step')
d keys 'DELETE THE OLD BUILD CACHE'
d click 'Run critical step'
Start-Sleep -Seconds 15
d capture '' 'held'
check 'the critical step ran once confirmed' (-not (Test-Path $cache))
check 'after the run, the stage is held' (-not (has 'Back to the plan'))

# Typed while held: must not reach the shell.
d keys 'New-Item -ItemType File C:\kj\work\leaked.txt'
d chord '{ENTER}'
Start-Sleep -Seconds 4
check 'keys typed while held went nowhere' (-not (Test-Path "$w\leaked.txt"))
d chord '^+k'
Start-Sleep -Seconds 3
d capture '' 'released'
check 'Ctrl+Shift+K lets go of the stage' (has 'Back to the plan')

Get-Process keyjutsu-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
if ($script:failed) { log 'RESULT: FAIL' } else { log 'RESULT: PASS' }
New-Item -ItemType File "$w\all.done" | Out-Null
