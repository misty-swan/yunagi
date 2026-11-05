# Yunagi installer for Windows.
#
#   irm https://<site>/install.ps1 | iex
#
# What it does: docs/release/07-build-and-distribution.md section 4.2.
# Environment variables (all optional):
#   YUNAGI_VERSION      version to install (default: the "version" in latest.json). "v1.2.3" and "1.2.3" both work.
#   YUNAGI_BASE_URL     where release files are (default: GitHub Releases). Files are read from <base>/v<version>/.
#   YUNAGI_LATEST_URL   where latest.json is (default: the official site).
#
# Keep this file ASCII only: Windows PowerShell 5.1 reads a file without a BOM as ANSI.
# Everything is in functions and the last line calls Install-Yunagi, so a download that is cut off
# halfway never runs a half-written command. Setting YUNAGI_INSTALL_NO_RUN=1 loads the functions
# without installing (the tests use this).

# These two are placeholders until the public repository and the site are decided (docs/release/07 section 4.1).
$script:DefaultBaseUrl = 'https://github.com/<public-repository>/releases/download'
$script:DefaultLatestUrl = 'https://<site>/latest.json'

function Stop-Install([string]$Message) {
    throw "install.ps1: $Message"
}

# Returns 'x86_64' or 'arm64'. Anything else stops the installation.
function Get-YunagiArch([string]$Architecture) {
    switch ($Architecture) {
        'X64' { return 'x86_64' }
        'Arm64' { return 'arm64' }
        default { Stop-Install "unsupported architecture: $Architecture (supported: x64, arm64)." }
    }
}

function Test-PlaceholderUrl([string]$Url, [string]$VariableName) {
    if ($Url.Contains('<')) {
        Stop-Install "the download location is not decided yet ($Url). Set $VariableName to a real URL."
    }
}

# Returns the version without a leading 'v'. Stops when it has an unexpected shape.
function ConvertTo-YunagiVersion([string]$Version) {
    $v = $Version.Trim()
    if ($v.StartsWith('v')) { $v = $v.Substring(1) }
    if ($v -notmatch '^[0-9][0-9A-Za-z.+-]*$') { Stop-Install "invalid version: $Version" }
    return $v
}

# Reads the hash from the text of a .sha256 file ("<hash>  <file name>" or just "<hash>").
function Read-Sha256File([string]$Text) {
    $first = ($Text.Trim() -split '\s+')[0].ToLowerInvariant()
    if ($first -notmatch '^[0-9a-f]{64}$') { Stop-Install 'the .sha256 file does not hold a SHA-256 hash.' }
    return $first
}

# Stops when the file's SHA-256 differs from the expected one.
function Assert-Sha256([string]$Path, [string]$Expected) {
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -ne $Expected) {
        Stop-Install "SHA-256 mismatch for $(Split-Path -Leaf $Path) (expected $Expected, got $actual). Nothing was installed."
    }
}

# Adds Dir to the user PATH when it is not there yet. Returns the new value, or $null when nothing changed.
function Get-PathWithEntry([string]$Current, [string]$Dir) {
    $entries = @()
    if ($Current) { $entries = $Current.Split(';') | Where-Object { $_ -ne '' } }
    $wanted = $Dir.TrimEnd('\')
    foreach ($e in $entries) {
        if ($e.TrimEnd('\') -ieq $wanted) { return $null }
    }
    return (@($entries) + $Dir) -join ';'
}

function Save-File([string]$Url, [string]$Path) {
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Path
    } catch {
        Stop-Install "could not download $Url ($($_.Exception.Message))"
    }
}

function Install-Yunagi {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue' # the progress bar makes Windows PowerShell 5.1 very slow
    # Windows PowerShell 5.1 may not offer TLS 1.2 by default
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $arch = Get-YunagiArch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString())

    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("yunagi-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $version = $env:YUNAGI_VERSION
        if (-not $version) {
            $latestUrl = if ($env:YUNAGI_LATEST_URL) { $env:YUNAGI_LATEST_URL } else { $script:DefaultLatestUrl }
            Test-PlaceholderUrl $latestUrl 'YUNAGI_LATEST_URL'
            $latestFile = Join-Path $tmp 'latest.json'
            Save-File $latestUrl $latestFile
            $version = (Get-Content -Raw -LiteralPath $latestFile | ConvertFrom-Json).version
            if (-not $version) { Stop-Install "could not read the version from $latestUrl" }
        }
        $version = ConvertTo-YunagiVersion $version
        Write-Host "Installing Yunagi $version (windows, $arch)"

        $base = if ($env:YUNAGI_BASE_URL) { $env:YUNAGI_BASE_URL } else { $script:DefaultBaseUrl }
        Test-PlaceholderUrl $base 'YUNAGI_BASE_URL'
        $name = "Yunagi_${version}_${arch}-setup.exe"
        $setup = Join-Path $tmp $name
        Save-File "$base/v$version/$name" $setup
        Save-File "$base/v$version/$name.sha256" "$setup.sha256"
        Assert-Sha256 $setup (Read-Sha256File (Get-Content -Raw -LiteralPath "$setup.sha256"))
        Write-Host "Verified $name"

        # NSIS: /S is silent, /D sets the install folder (it must be the last argument, without quotes).
        $installDir = Join-Path $env:LOCALAPPDATA 'Programs\Yunagi'
        $process = Start-Process -FilePath $setup -ArgumentList @('/S', "/D=$installDir") -Wait -PassThru
        if ($process.ExitCode -ne 0) { Stop-Install "the installer failed (exit code $($process.ExitCode))." }

        $binDir = Join-Path $installDir 'bin'
        if (-not (Test-Path -LiteralPath (Join-Path $binDir 'yunagi.exe'))) {
            Stop-Install "the installer finished but $binDir\yunagi.exe was not found."
        }

        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $newPath = Get-PathWithEntry $userPath $binDir
        if ($newPath) {
            [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        }
        if ($null -ne (Get-PathWithEntry $env:Path $binDir)) {
            $env:Path = "$env:Path;$binDir"
        }

        Write-Host ''
        Write-Host "Installed Yunagi ${version}:"
        Write-Host "  app            $installDir\Yunagi.exe"
        Write-Host "  command        $binDir\yunagi.exe (added to your user PATH)"
        Write-Host '  Start menu     Yunagi'
        if ($newPath) {
            Write-Host 'Open a new terminal to use the yunagi command.'
        }
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

if (-not $env:YUNAGI_INSTALL_NO_RUN) { Install-Yunagi }
