<#
.SYNOPSIS
    Signs a Windows file with Azure Trusted Signing. Tauri calls it once for
    each file it bundles: the CLI and the broker, the desktop app, and the
    installer and its uninstaller.

.DESCRIPTION
    Public trust without a private key on the build machine. The certificate
    lives in Azure; signtool reaches it through Microsoft's signing dlib, and
    authentication is the OIDC token azure/login already exchanged. There is no
    PFX to store, leak or rotate.

    Signing is opt-in and driven entirely by environment. With
    ARTIFACT_SIGNING_ACCOUNT unset this says so and exits zero, so a build
    without signing configured produces unsigned files rather than failing.

    Half-configured is an error rather than a fallback: an account with no
    endpoint or profile means somebody set one secret and not the rest, and
    quietly shipping unsigned would hide it until a user saw the warning
    Windows shows for unknown publishers.

.PARAMETER Path
    The file to sign. It is signed, then verified.
#>
param(
    [Parameter(Mandatory)]
    [string] $Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$account  = $env:ARTIFACT_SIGNING_ACCOUNT
$endpoint = $env:ARTIFACT_SIGNING_ENDPOINT
$profile  = $env:ARTIFACT_SIGNING_PROFILE

if (-not $account) {
    Write-Host "Signing is not configured; leaving $(Split-Path $Path -Leaf) unsigned."
    return
}

foreach ($pair in @{ ARTIFACT_SIGNING_ENDPOINT = $endpoint; ARTIFACT_SIGNING_PROFILE = $profile }.GetEnumerator()) {
    if (-not $pair.Value) {
        throw "$($pair.Key) is empty but ARTIFACT_SIGNING_ACCOUNT is set: refusing to sign half-configured."
    }
}

if (-not (Test-Path $Path)) { throw "There is nothing to sign at '$Path'." }
$file = (Resolve-Path $Path).Path

# Kept across calls within one job: Tauri calls this once per file, and
# fetching the client each time would multiply a slow step for nothing.
$tools = Join-Path ($env:RUNNER_TEMP ?? $env:TEMP) 'keyjutsu-signing'
New-Item -ItemType Directory -Force -Path $tools | Out-Null

$dlib = Get-ChildItem $tools -Recurse -Filter 'Azure.CodeSigning.Dlib.dll' -ErrorAction SilentlyContinue |
        Select-Object -First 1
if (-not $dlib) {
    Write-Host 'Fetching the Trusted Signing client...'
    & nuget install Microsoft.Trusted.Signing.Client -Version 1.0.53 -OutputDirectory $tools -Verbosity quiet
    if ($LASTEXITCODE -ne 0) { throw 'nuget install of Microsoft.Trusted.Signing.Client failed.' }
    $dlib = Get-ChildItem $tools -Recurse -Filter 'Azure.CodeSigning.Dlib.dll' | Select-Object -First 1
}
if (-not $dlib) { throw 'Azure.CodeSigning.Dlib.dll was not found after installing the client.' }

# ExcludeCredentials is not tuning, it is the difference between signing and
# hanging. The dlib tries each credential type in turn, and the managed
# identity one probes an address that, off Azure, stalls rather than refusing:
# "Submitting digest for signing..." and then nothing, with no error and no
# timeout. azure/login fills in the Azure CLI, so that is the one kept.
$metadata = Join-Path $tools 'metadata.json'
@{
    Endpoint               = $endpoint
    CodeSigningAccountName = $account
    CertificateProfileName = $profile
    ExcludeCredentials     = @(
        'ManagedIdentityCredential',
        'WorkloadIdentityCredential',
        'SharedTokenCacheCredential',
        'VisualStudioCredential',
        'VisualStudioCodeCredential',
        'AzurePowerShellCredential',
        'AzureDeveloperCliCredential',
        'InteractiveBrowserCredential'
    )
} | ConvertTo-Json | Set-Content -Path $metadata -Encoding utf8

$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -like '*x64*' } |
            Select-Object -Last 1
if (-not $signtool) { throw 'signtool.exe was not found. Install the Windows SDK.' }

Write-Host "Signing $(Split-Path $file -Leaf)..."
# The timestamp is what keeps a signature valid after the certificate expires.
# Without it every release stops verifying the day the certificate does.
& $signtool.FullName sign /v /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 `
    /dlib $dlib.FullName /dmdf $metadata $file
if ($LASTEXITCODE -ne 0) { throw "signtool failed for '$file' with exit code $LASTEXITCODE." }

# Verified rather than assumed: a signature that doesn't chain can still be
# reported as a success, and a release is the wrong place to find out.
& $signtool.FullName verify /pa /v $file
if ($LASTEXITCODE -ne 0) { throw "The signature on '$file' did not verify." }
Write-Host "Signed and verified $(Split-Path $file -Leaf) with profile $profile."
