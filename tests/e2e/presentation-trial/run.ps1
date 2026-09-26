# "Keep the illusion" in the desktop app, end to end, in Windows Sandbox:
# the Sandbox's clock is moved on to age an approval, and the app is driven
# there, so no click or key reaches your desktop.
#
#   pwsh tests/e2e/presentation-trial/run.ps1
#
# Uses the installer from `pnpm desktop:build` (build it first). Networking is
# on because the installer fetches WebView2. Prints the log and leaves the
# screenshots in the staging folder.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-presentation-trial"
if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw "Windows Sandbox is already running; close it first" }
$installer = Get-ChildItem "$repo\target\release\bundle\nsis\*setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $installer) { throw "no installer: run pnpm desktop:build first" }
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\installer", "$stage\work" | Out-Null
Copy-Item $installer.FullName "$stage\installer\"
Copy-Item "$repo\tests\e2e\drive-desktop.ps1", "$PSScriptRoot\trial.ps1", "$repo\docs\examples\clear-build-cache.json" "$stage\"

@"
<Configuration>
  <Networking>Enable</Networking>
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
$deadline = (Get-Date).AddMinutes(25)
while (-not (Test-Path "$stage\work\all.done")) {
    if ((Get-Date) -gt $deadline) { throw "no result after 25 minutes; see $stage\work\trial.log" }
    Start-Sleep -Seconds 5
}
# The Sandbox is this script's own (it refuses to start beside another), and
# left open it stops the next run from starting.
Get-Process WindowsSandboxRemoteSession, WindowsSandboxServer -ErrorAction SilentlyContinue | Stop-Process -Force
Get-Content "$stage\work\trial.log"
"Screenshots in $stage\work"
if (-not (Select-String -LiteralPath "$stage\work\trial.log" -Pattern 'RESULT: PASS' -Quiet)) { exit 1 }
