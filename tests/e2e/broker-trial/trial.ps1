# Runs inside Windows Sandbox only: real elevation through KeyJutsu's broker.
#
# The Sandbox runs with UAC off, so everything there is already elevated. To
# cross a real elevation boundary, the first start turns UAC on (with consent
# set to elevate without prompting, so nobody has to click), restarts the
# Sandbox, and the trial then runs as an ordinary, unelevated user whose
# broker is elevated by UAC.
$kj = 'C:\kj'
$w = "$kj\work"
New-Item -ItemType Directory -Force $w | Out-Null
function log($m) { Add-Content -LiteralPath "$w\trial.log" -Value ("{0} {1}" -f (Get-Date -Format o), $m) }

if ($env:USERNAME -ne 'WDAGUtilityAccount') { log "not in Windows Sandbox ($env:USERNAME); stopping"; exit 1 }
if (Test-Path "$w\started") { log "launcher ran again; nothing to do"; exit }

$policies = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'
$lua = (Get-ItemProperty $policies -Name EnableLUA -ErrorAction SilentlyContinue).EnableLUA
log "UAC (EnableLUA): $lua"
log ("integrity: " + ((whoami /groups | Select-String 'Mandatory Label') -replace '\s+', ' '))

if ($lua -ne 1 -and -not (Test-Path "$w\uac.on")) {
    Set-ItemProperty $policies -Name EnableLUA -Value 1
    Set-ItemProperty $policies -Name ConsentPromptBehaviorAdmin -Value 0
    Set-ItemProperty $policies -Name PromptOnSecureDesktop -Value 0
    New-Item -ItemType File "$w\uac.on" | Out-Null
    reg.exe add 'HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce' /v KeyJutsuBrokerTrial /t REG_SZ /d "powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File $kj\trial.ps1" /f | Out-Null
    log "UAC turned on; restarting the Sandbox so it takes effect"
    shutdown.exe /r /t 5 /c "KeyJutsu broker trial: turning UAC on"
    exit
}

New-Item -ItemType File "$w\started" | Out-Null
& "$kj\bin\broker_trial.exe" $w *>> "$w\trial.log"
log "broker trial exited $LASTEXITCODE"
New-Item -ItemType File "$w\all.done" | Out-Null
