# Runs inside Windows Sandbox only: KeyJutsu on a clean Windows 11.
# Install silently, check what the installer did, run the
# readiness scan and agent detection, drive the safe demo in Performance
# mode, uninstall, and check it all went.
$kj = 'C:\kj'
$w = "$kj\work"
New-Item -ItemType Directory -Force $w | Out-Null
function log($m) { Add-Content -LiteralPath "$w\trial.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }
function check($name, $ok) { log ("{0} {1}" -f ($(if ($ok) { 'ok  ' } else { 'FAIL' })), $name); if (-not $ok) { $script:failed = $true } }

if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }
if (Test-Path "$w\started") { log "launcher ran again; nothing to do"; exit }
New-Item -ItemType File "$w\started" | Out-Null
$failed = $false

# What a clean machine has.
log ("Windows: " + (Get-CimInstance Win32_OperatingSystem).Caption + " " + (Get-CimInstance Win32_OperatingSystem).BuildNumber)
log ("Visual C++ runtime present: " + (Test-Path "$env:windir\System32\vcruntime140.dll"))
log ("PowerShell 7 present: " + [bool](Get-Command pwsh -ErrorAction SilentlyContinue))
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
log ("WebView2 runtime: " + $(if ($wv) { $wv.pv } else { 'not found' }))

# Install, silently (both questions answered yes).
$installer = Get-ChildItem "$kj\installer\*.exe" | Select-Object -First 1
log "installing $($installer.Name)"
$p = Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait -PassThru
check "installer exited 0 ($($p.ExitCode))" ($p.ExitCode -eq 0)
$dir = "$env:ProgramFiles\KeyJutsu"
if (-not (Test-Path "$dir\keyjutsu.exe")) {
    # Nothing after this means anything without an installation.
    log "not installed; stopping"
    log "RESULT: FAIL"
    New-Item -ItemType File "$w\all.done" | Out-Null
    exit 1
}
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
log ("WebView2 runtime after installing: " + $(if ($wv) { $wv.pv } else { 'not found' }))
foreach ($f in 'keyjutsu-desktop.exe', 'keyjutsu.exe', 'keyjutsu-broker.exe', 'uninstall.exe') {
    check "installed $f" (Test-Path "$dir\$f")
}
$userPath = (Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
check "install folder on PATH ($userPath)" ($userPath -split ';' -contains $dir)
check "Explorer folder menu" (Test-Path 'HKCU:\Software\Classes\Directory\shell\KeyJutsu\command')
check "Explorer background menu" (Test-Path 'HKCU:\Software\Classes\Directory\Background\shell\KeyJutsu\command')

# The CLI as a new terminal would find it: on the PATH.
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
$cli = (Get-Command keyjutsu -ErrorAction SilentlyContinue).Source
check "keyjutsu found on PATH ($cli)" ([bool]$cli)
log "--- keyjutsu doctor"
& keyjutsu doctor *>> "$w\trial.log"
check "doctor exited ($LASTEXITCODE)" ($LASTEXITCODE -eq 0 -or $LASTEXITCODE -eq 1)
log "--- keyjutsu agents (clean machine)"
$none = (& keyjutsu agents 2>&1) -join "`n"
log $none
check "agent detection ran ($LASTEXITCODE)" ($LASTEXITCODE -eq 0)
# The Sandbox has no network to install a real agent, so a stand-in answers
# `--version` the way Codex does. Detection finding it proves the search of
# the PATH and the version check; the live checks against real agents are in
# docs/agent-integrations.
New-Item -ItemType Directory -Force "$kj\agents" | Out-Null
Set-Content -LiteralPath "$kj\agents\codex.cmd" -Value '@echo codex-cli 0.154.0'
$env:Path = "$kj\agents;$env:Path"
log "--- keyjutsu agents (a stand-in Codex on the PATH)"
$found = (& keyjutsu agents 2>&1) -join "`n"
log $found
check "no agents on a clean machine" ($none -match 'Codex CLI\s+not installed' -and $none -match 'Claude Code\s+not installed')
check "stand-in Codex detected with its version" ($found -match 'Codex CLI\s+version 0\.154\.0')

# The desktop app starts (WebView2 and all) and stays up.
$app = Start-Process -FilePath "$dir\keyjutsu-desktop.exe" -PassThru
Start-Sleep -Seconds 8
$alive = $null -ne $app -and -not $app.HasExited
check "desktop app running after 8 s" $alive
if ($alive) { Stop-Process -Id $app.Id }

# The safe demo, in Performance mode, typed by mashed keys.
log "--- safe demo"
& "$kj\bin\demo_trial.exe" $cli *>> "$w\trial.log"
check "demo trial ($LASTEXITCODE)" ($LASTEXITCODE -eq 0)

# Uninstall, silently, and check it all went, including what the broker
# keeps in ProgramData (made here as it would be, since no plan has run).
New-Item -ItemType Directory -Force "$env:ProgramData\KeyJutsu\captures\trial" | Out-Null
$u = Start-Process -FilePath "$dir\uninstall.exe" -ArgumentList '/S' -Wait -PassThru
Start-Sleep -Seconds 5
check "uninstaller exited 0 ($($u.ExitCode))" ($u.ExitCode -eq 0)
check "program files removed" (-not (Test-Path "$dir\keyjutsu.exe"))
check "broker captures removed" (-not (Test-Path "$env:ProgramData\KeyJutsu"))
$userPath = (Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
check "install folder off PATH" (-not ($userPath -split ';' -contains $dir))
check "Explorer menus removed" (-not (Test-Path 'HKCU:\Software\Classes\Directory\shell\KeyJutsu') -and -not (Test-Path 'HKCU:\Software\Classes\Directory\Background\shell\KeyJutsu'))

if ($failed) { log "RESULT: FAIL" } else { log "RESULT: PASS" }
New-Item -ItemType File "$w\all.done" | Out-Null
