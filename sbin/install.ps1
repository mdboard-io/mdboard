# Install the mdBoard CLI into %USERPROFILE%\.mdboard\bin.
# Usage: irm https://mdboard.io/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$repo = 'mdboard-io/mdboard'
$installDir = if ($env:MDBOARD_INSTALL) { $env:MDBOARD_INSTALL } else { Join-Path $HOME '.mdboard' }
$binDir = Join-Path $installDir 'bin'
$exe = Join-Path $binDir 'mdboard.exe'
$version = $env:MDBOARD_VERSION
$fromSource = $env:MDBOARD_FROM_SOURCE -eq '1'
$channel = if ($env:MDB_CHANNEL) { $env:MDB_CHANNEL } else { 'stable' }

function Write-Info($message) { Write-Host $message -ForegroundColor DarkGray }
function Write-Ok($message) { Write-Host $message -ForegroundColor Green }
function Fail($message) { Write-Error "error: $message"; exit 1 }

if ($channel -notin @('stable', 'nightly')) { Fail "Unknown channel: $channel (use stable or nightly)" }

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($arch -ne [System.Runtime.InteropServices.Architecture]::X64) {
  Fail "Unsupported Windows architecture: $arch. Only x64 is currently packaged."
}
$target = 'x86_64-pc-windows-msvc'

function Get-Tag {
  if ($version) {
    if ($version.StartsWith('v')) { return $version }
    return "v$version"
  }
  try {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'mdboard-installer' }
    return $release.tag_name
  } catch { return $null }
}

function Install-Release($tag) {
  $asset = "mdboard-$tag-$target.zip"
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mdboard-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tmp | Out-Null
  try {
    Write-Info "Downloading $asset"
    $url = "https://github.com/$repo/releases/download/$tag/$asset"
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp $asset) -UseBasicParsing
    $sumsPath = Join-Path $tmp 'SHA256SUMS'
    try {
      Invoke-WebRequest -Uri "https://github.com/$repo/releases/download/$tag/SHA256SUMS" -OutFile $sumsPath -UseBasicParsing
      $line = Get-Content $sumsPath | Where-Object { $_ -match "\s+$([regex]::Escape($asset))$" } | Select-Object -First 1
      if (-not $line) { Fail "No checksum found for $asset" }
      $expected = ($line -split '\s+')[0].ToLowerInvariant()
      $actual = (Get-FileHash (Join-Path $tmp $asset) -Algorithm SHA256).Hash.ToLowerInvariant()
      if ($actual -ne $expected) { Fail "Checksum failed for $asset" }
    } catch {
      Fail "Could not verify SHA256SUMS for $tag"
    }
    Expand-Archive -Path (Join-Path $tmp $asset) -DestinationPath $tmp -Force
    $src = Join-Path $tmp "mdboard-$tag-$target\mdboard.exe"
    if (-not (Test-Path $src)) { Fail 'Archive did not contain mdboard.exe' }
    Copy-Item $src $exe -Force
  } finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
}

function Install-FromSource {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail 'No published release found and cargo is not installed.'
  }
  Write-Info "No usable release found; building the $channel branch from source"
  $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("mdboard-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tmp | Out-Null
  try {
    $archive = Join-Path $tmp 'mdboard.tar.gz'
    Invoke-WebRequest -Uri "https://github.com/$repo/archive/refs/heads/$channel.tar.gz" -OutFile $archive -UseBasicParsing
    tar -xzf $archive -C $tmp
    $root = Join-Path $tmp "mdboard-$channel"
    if (-not (Test-Path $root)) { Fail "Could not unpack the $channel source archive" }
    Push-Location $root
    try { cargo build --locked --release -p mdboard-cli } finally { Pop-Location }
    Copy-Item (Join-Path $root 'target\release\mdboard.exe') $exe -Force
  } finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
  }
}

New-Item -ItemType Directory -Path $binDir -Force | Out-Null

$tag = $null
if (-not $fromSource -and $channel -eq 'stable') { $tag = Get-Tag }
if ($tag) { Install-Release $tag } else { Install-FromSource }

Write-Ok "mdboard installed to $exe"
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $userPath) { $userPath = '' }
if (($userPath -split ';') -notcontains $binDir) {
  $newPath = if ($userPath) { "$userPath;$binDir" } else { $binDir }
  [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
  $env:Path = "$env:Path;$binDir"
  Write-Info "Added $binDir to the user PATH. Restart your terminal."
}
Write-Host ''
Write-Host "Run mdboard --help to get started."
