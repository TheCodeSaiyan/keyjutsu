# Real elevation through KeyJutsu's broker, in Windows Sandbox
# so that the Administrator step (a key under HKLM) lands on a disposable
# machine.
#
#   pwsh tests/e2e/broker-trial/run.ps1
#
# Builds the broker and examples/broker_trial.rs with a static C runtime,
# stages them with trial.ps1, starts Sandbox with networking off and only the
# staging folder mapped, and prints the log. If the Sandbox shows a UAC
# prompt, answer it there. Close the Sandbox window afterwards.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-broker-trial"
if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw "Windows Sandbox is already running; close it first" }
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\bin", "$stage\work" | Out-Null

$env:CARGO_TARGET_DIR = Join-Path $repo 'target\static'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release -p keyjutsu-broker --bins --examples --manifest-path "$repo\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Copy-Item "$repo\target\static\release\keyjutsu-broker.exe", "$repo\target\static\release\examples\broker_trial.exe" "$stage\bin\"
Copy-Item "$PSScriptRoot\trial.ps1" "$stage\"

@"
<Configuration>
  <Networking>Disable</Networking>
  <vGPU>Disable</vGPU>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <MappedFolders>
    <MappedFolder>
      <HostFolder>$stage</HostFolder>
      <SandboxFolder>C:\kj</SandboxFolder>
      <ReadOnly>false</ReadOnly>
    </MappedFolder>
  </MappedFolders>
  <LogonCommand>
    <Command>powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kj\trial.ps1</Command>
  </LogonCommand>
</Configuration>
"@ | Set-Content -LiteralPath "$stage\trial.wsb"

Start-Process "$stage\trial.wsb"
$deadline = (Get-Date).AddMinutes(10)
while (-not (Test-Path "$stage\work\all.done")) {
    if ((Get-Date) -gt $deadline) { throw "no result after 10 minutes; see $stage\work\trial.log" }
    Start-Sleep -Seconds 5
}
# The Sandbox is this script's own (it refuses to start beside another), and
# left open it stops the next run from starting.
Get-Process WindowsSandboxRemoteSession, WindowsSandboxServer -ErrorAction SilentlyContinue | Stop-Process -Force
# The log mixes UTF-8 (Add-Content) and UTF-16 (the program's redirected
# output); dropping NULs makes both readable and searchable.
$log = ([IO.File]::ReadAllText("$stage\work\trial.log")) -replace "`0", ""
$log
if ($log -match '(?m)^PASS:') { "BROKER TRIAL PASSED" } else { throw "BROKER TRIAL FAILED" }
