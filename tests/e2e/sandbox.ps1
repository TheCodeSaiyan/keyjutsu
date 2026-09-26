# Starting a Windows Sandbox trial, and noticing when it did not start.
#
#   . "$PSScriptRoot\..\sandbox.ps1"
#   Start-SandboxTrial -Wsb "$stage\trial.wsb" -Watch "$stage\work"
#
# Windows Sandbox fails in two quiet ways on a machine that has just closed
# one: it does not start at all, or it starts and never runs the .wsb file's
# logon command. Either way the trial's folder stays empty and the script
# would wait out its whole deadline. This checks, within a minute, that the
# Sandbox is there, and within a few more that the trial has written its
# first file; if not, it closes the Sandbox, waits for it to be gone, and
# tries once more before giving up.

function Stop-Sandbox {
    Get-Process WindowsSandboxRemoteSession, WindowsSandboxServer -ErrorAction SilentlyContinue | Stop-Process -Force
    $until = (Get-Date).AddSeconds(120)
    while ((Get-Process WindowsSandboxRemoteSession, WindowsSandboxServer, vmmemWindowsSandbox -ErrorAction SilentlyContinue) -and (Get-Date) -lt $until) {
        Start-Sleep -Seconds 2
    }
    # Gone is not the same as ready for the next one.
    Start-Sleep -Seconds 30
}

function Start-SandboxTrial {
    param(
        [Parameter(Mandatory)] [string] $Wsb,
        # The folder the trial writes into; any file there means it began.
        [Parameter(Mandatory)] [string] $Watch,
        [int] $StartMinutes = 5
    )
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        Start-Process $Wsb
        $until = (Get-Date).AddSeconds(90)
        while (-not (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) -and (Get-Date) -lt $until) {
            Start-Sleep -Seconds 2
        }
        if (Get-Process WindowsSandboxRemoteSession -ErrorAction SilentlyContinue) {
            $until = (Get-Date).AddMinutes($StartMinutes)
            while (-not (Get-ChildItem -LiteralPath $Watch -ErrorAction SilentlyContinue) -and (Get-Date) -lt $until) {
                Start-Sleep -Seconds 5
            }
            if (Get-ChildItem -LiteralPath $Watch -ErrorAction SilentlyContinue) { return }
            Write-Host "The Sandbox started but did not run the trial (attempt $attempt); closing it."
        } else {
            Write-Host "The Sandbox did not start (attempt $attempt)."
        }
        Stop-Sandbox
    }
    throw "Windows Sandbox did not run the trial twice running; close any Sandbox and run this again"
}
