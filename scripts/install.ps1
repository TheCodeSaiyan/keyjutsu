<#
.SYNOPSIS
    Downloads a KeyJutsu release, checks it, and runs its installer.

.DESCRIPTION
    Published beside every release, so this is the whole install:

        irm https://github.com/TheCodeSaiyan/keyjutsu/releases/latest/download/install.ps1 | iex

    It downloads the installer and the release's SHA256SUMS, and refuses to go
    on if the installer's hash isn't the one listed, so a damaged or altered
    download is never run. It then checks the installer's signature: a
    signature that is present but doesn't verify is refused, and an unsigned
    installer is said out loud rather than run quietly. Then the installer
    runs as it would if you double-clicked it: Windows asks for Administrator,
    and it asks its two questions.

    Written for Windows PowerShell 5.1 as well as PowerShell 7, because 5.1 is
    all a new Windows has.

.PARAMETER Version
    The version to install, such as 0.2.0. The latest release otherwise.

.PARAMETER Silent
    Install without asking, answering yes to both questions.

.EXAMPLE
    & ([scriptblock]::Create((irm https://github.com/TheCodeSaiyan/keyjutsu/releases/latest/download/install.ps1))) -Version 0.2.0
#>
param(
    [string] $Version,
    [switch] $Silent
)

$ErrorActionPreference = 'Stop'
$repository = 'TheCodeSaiyan/keyjutsu'

# Windows PowerShell 5.1 may still offer TLS 1.0 first, which GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$api = if ($Version) {
    "https://api.github.com/repos/$repository/releases/tags/v$($Version.TrimStart('v'))"
} else {
    "https://api.github.com/repos/$repository/releases/latest"
}
$release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'keyjutsu-install' }
$installer = $release.assets | Where-Object { $_.name -like 'KeyJutsu_*_x64-setup.exe' } | Select-Object -First 1
$sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
if (-not $installer -or -not $sums) {
    throw "Release $($release.tag_name) has no installer or no SHA256SUMS; nothing was installed."
}

$work = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    Write-Host "Downloading KeyJutsu $($release.tag_name)..."
    $exe = Join-Path $work $installer.name
    Invoke-WebRequest -Uri $installer.browser_download_url -OutFile $exe -UseBasicParsing
    Invoke-WebRequest -Uri $sums.browser_download_url -OutFile (Join-Path $work 'SHA256SUMS') -UseBasicParsing

    $listed = Get-Content (Join-Path $work 'SHA256SUMS') |
        Where-Object { $_ -match "^([0-9a-fA-F]{64})\s+\*?$([regex]::Escape($installer.name))$" } |
        ForEach-Object { $Matches[1].ToLowerInvariant() } | Select-Object -First 1
    if (-not $listed) { throw "SHA256SUMS does not list $($installer.name); nothing was installed." }
    $actual = (Get-FileHash -Algorithm SHA256 $exe).Hash.ToLowerInvariant()
    if ($actual -ne $listed) {
        throw "$($installer.name) does not match SHA256SUMS (expected $listed, got $actual); nothing was installed."
    }
    Write-Host "Checksum matches SHA256SUMS."

    $signature = Get-AuthenticodeSignature $exe
    switch ($signature.Status) {
        'Valid' { Write-Host "Signed by $($signature.SignerCertificate.Subject)." }
        'NotSigned' {
            Write-Warning 'This installer is not signed, so Windows will warn about an unknown publisher. Its checksum matched the release.'
        }
        default { throw "The installer's signature does not verify ($($signature.Status)); nothing was installed." }
    }

    $arguments = if ($Silent) { @('/S') } else { @() }
    $run = Start-Process -FilePath $exe -ArgumentList $arguments -Wait -PassThru
    if ($run.ExitCode -ne 0) { throw "The installer exited with $($run.ExitCode)." }
    Write-Host 'Installed. Open a new terminal and run: keyjutsu doctor'
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
