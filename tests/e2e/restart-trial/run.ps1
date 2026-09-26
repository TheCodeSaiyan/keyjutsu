# A real Windows restart across a plan boundary, in Windows
# Sandbox so that nothing but a disposable machine restarts.
#
#   pwsh tests/e2e/restart-trial/run.ps1
#
# Builds examples/boundary_trial.rs with a static C runtime (Sandbox may not
# have the Visual C++ runtime), stages it with trial.ps1 in a temporary
# folder, starts Sandbox with networking off and only that folder mapped,
# and prints the log when the trial is done. trial.ps1 refuses to run outside
# the Sandbox's own account. Close the Sandbox window afterwards.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-restart-trial"
if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw "Windows Sandbox is already running; close it first" }
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\bin", "$stage\work" | Out-Null

$env:CARGO_TARGET_DIR = Join-Path $repo 'target\static'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release -p keyjutsu-core --example boundary_trial --manifest-path "$repo\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Copy-Item "$repo\target\static\release\examples\boundary_trial.exe" "$stage\bin\"
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

. "$PSScriptRoot\..\sandbox.ps1"
Start-SandboxTrial -Wsb "$stage\trial.wsb" -Watch "$stage\work"
$deadline = (Get-Date).AddMinutes(15)
while (-not (Test-Path "$stage\work\all.done")) {
    if ((Get-Date) -gt $deadline) { throw "no result after 15 minutes; see $stage\work\trial.log" }
    Start-Sleep -Seconds 5
}
# The log mixes UTF-8 (Add-Content) and UTF-16 (the program's redirected
# output); dropping NULs makes both readable and searchable.
$log = ([IO.File]::ReadAllText("$stage\work\trial.log")) -replace "`0", ""
$log
if ($log -match 'PASS phase 2') { "RESTART TRIAL PASSED" } else { throw "RESTART TRIAL FAILED" }
