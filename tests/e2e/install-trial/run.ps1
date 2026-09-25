# KeyJutsu on a clean Windows 11 (Milestone 16), in Windows Sandbox.
#
#   pwsh tests/e2e/install-trial/run.ps1
#
# Uses the installer from `pnpm desktop:build` (build it first) and a static
# build of the demo driver, starts Sandbox with only the staging folder
# mapped, and prints the log. Close the Sandbox afterwards.
#
# Networking is on, unlike the other trials: Sandbox has no WebView2 runtime,
# and the installer fetches it from Microsoft, as it would on any machine
# without one. With networking off the installer stops (exit code 2).
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-install-trial"
if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw "Windows Sandbox is already running; close it first" }
$installer = Get-ChildItem "$repo\target\release\bundle\nsis\*setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $installer) { throw "no installer: run pnpm desktop:build first" }
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\bin", "$stage\work", "$stage\installer" | Out-Null

cargo build --release -p keyjutsu-cli --example demo_trial --manifest-path "$repo\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Copy-Item "$repo\target\release\examples\demo_trial.exe" "$stage\bin\"
Copy-Item $installer.FullName "$stage\installer\"
Copy-Item "$PSScriptRoot\trial.ps1" "$stage\"

@"
<Configuration>
  <Networking>Enable</Networking>
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
$deadline = (Get-Date).AddMinutes(20)
while (-not (Test-Path "$stage\work\all.done")) {
    if ((Get-Date) -gt $deadline) { throw "no result after 20 minutes; see $stage\work\trial.log" }
    Start-Sleep -Seconds 5
}
# The log mixes UTF-8 (Add-Content) and UTF-16 (redirected output).
$log = ([IO.File]::ReadAllText("$stage\work\trial.log")) -replace "`0", ""
$log
if ($log -match 'RESULT: PASS') { "INSTALL TRIAL PASSED" } else { throw "INSTALL TRIAL FAILED" }
