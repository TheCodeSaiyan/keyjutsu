<#
.SYNOPSIS
    Packs the portable build: the KeyJutsu app and CLI in a zip, to try
    without installing.

.DESCRIPTION
    The release workflow runs this on the programs the installer has just
    installed and checked, so the zip holds exactly those files, signed or
    not as they were. Run it locally on a build to try the same thing:

        pwsh scripts/portable.ps1 -From target/release -Version 0.1.0 -Out out

    The elevation broker is left out on purpose. It is started as
    Administrator, and a portable folder is usually one the user, and so
    anything running as the user, can write to: whoever replaced the file
    would be handed Administrator at the next UAC prompt. That is why the
    installer installs into Program Files. Without it, KeyJutsu refuses
    Administrator steps and says why.

.PARAMETER From
    The folder holding keyjutsu-desktop.exe and keyjutsu.exe.

.PARAMETER Version
    The version, for the zip's name.

.PARAMETER Out
    Where to write KeyJutsu_<version>_x64-portable.zip.
#>
param(
    [Parameter(Mandatory)] [string] $From,
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $Out
)

$ErrorActionPreference = 'Stop'
$repo = Resolve-Path "$PSScriptRoot\.."
$programs = 'keyjutsu-desktop.exe', 'keyjutsu.exe'
foreach ($p in $programs) {
    if (-not (Test-Path (Join-Path $From $p))) { throw "no $p in $From; nothing was packed" }
}

$stage = Join-Path ([IO.Path]::GetTempPath()) "keyjutsu-portable-$([guid]::NewGuid().ToString('N'))"
$folder = Join-Path $stage "KeyJutsu-$Version"
New-Item -ItemType Directory -Force $folder | Out-Null
try {
    foreach ($p in $programs) { Copy-Item (Join-Path $From $p) $folder }
    Copy-Item (Join-Path $repo 'LICENSE') $folder
    @"
KeyJutsu $Version, portable

Nothing to install. Unzip this folder anywhere you can write to and run
keyjutsu-desktop.exe, or keyjutsu.exe from a terminal.

What the installer does and this doesn't:

- It can't run Administrator steps. They go through KeyJutsu's elevation
  broker, which runs as Administrator, so it's only installed where only
  Administrators can replace it: Program Files. A plan with Administrator
  steps is validated here, and refused before it runs.
- It doesn't put keyjutsu on your PATH or add "Open KeyJutsu here" to
  Explorer. "keyjutsu setup path add" and "keyjutsu setup explorer add" do
  both for your Windows account, and "remove" undoes them.
- There's no Start menu entry and no uninstaller: delete the folder.

What it keeps is the same as an installed copy: history, Techniques and
staged downloads under %LOCALAPPDATA%\KeyJutsu, encrypted where the
installed copy encrypts them. "keyjutsu store clear" removes them.

Windows 11, x64. Documentation: https://github.com/TheCodeSaiyan/keyjutsu
"@ | Set-Content -LiteralPath (Join-Path $folder 'README.txt') -Encoding utf8

    New-Item -ItemType Directory -Force $Out | Out-Null
    $zip = Join-Path (Resolve-Path $Out) "KeyJutsu_${Version}_x64-portable.zip"
    Remove-Item -LiteralPath $zip -ErrorAction SilentlyContinue
    Compress-Archive -Path $folder -DestinationPath $zip
    "Packed $zip"
} finally {
    Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
}
