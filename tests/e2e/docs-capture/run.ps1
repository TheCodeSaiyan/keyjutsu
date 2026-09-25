# The documentation's example output and screenshots, regenerated on a
# clean Windows 11 in Windows Sandbox rather than on anyone's own machine.
#
#   pwsh tests/e2e/docs-capture/run.ps1
#
# Uses the installer from `pnpm desktop:build` (build it first). Networking is
# on because the installer fetches WebView2, which Sandbox lacks. The app is
# driven inside the Sandbox, so no click or key ever reaches your desktop.
# Results land in the staging folder's out\; review them before copying the
# images into docs/images and the text into the pages.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\..\..\.."
$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-docs-capture"
if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) { throw "Windows Sandbox is already running; close it first" }
$installer = Get-ChildItem "$repo\target\release\bundle\nsis\*setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $installer) { throw "no installer: run pnpm desktop:build first" }
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\installer", "$stage\examples", "$stage\out" | Out-Null
Copy-Item $installer.FullName "$stage\installer\"
Copy-Item "$repo\docs\examples\*.json" "$stage\examples\"
Copy-Item "$repo\tests\e2e\drive-desktop.ps1", "$PSScriptRoot\capture.ps1" "$stage\"

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
    <Command>powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\kj\capture.ps1</Command>
  </LogonCommand>
</Configuration>
"@ | Set-Content -LiteralPath "$stage\capture.wsb"

Start-Process "$stage\capture.wsb"
$deadline = (Get-Date).AddMinutes(25)
while (-not (Test-Path "$stage\out\all.done")) {
    if ((Get-Date) -gt $deadline) { throw "no result after 25 minutes; see $stage\out\capture.log" }
    Start-Sleep -Seconds 5
}
Get-Content "$stage\out\capture.log"
"Results in $stage\out"
