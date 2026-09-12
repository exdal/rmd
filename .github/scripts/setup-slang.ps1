[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('windows-x86_64', 'linux-x86_64')]
    [string] $Platform
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$version = '2026.13.1'

$archives = @{
    'windows-x86_64' = @{
        Name = "slang-$version-windows-x86_64.zip"
        Sha256 = 'fa1c9bcab2cdcd3626f7a1e250dd35d606c1b84745b64627f1dd63fca3746a70'
    }
    'linux-x86_64' = @{
        Name = "slang-$version-linux-x86_64-glibc-2.27.tar.gz"
        Sha256 = 'f6db08763e38c398086d2b1d785ab7fc190bad27f29992e1be6ca3cc187884d0'
    }
}

$archive = $archives[$Platform]
$downloadUrl = "https://github.com/shader-slang/slang/releases/download/v$version/$($archive.Name)"
$temporaryRoot = if ($env:RUNNER_TEMP) {
    $env:RUNNER_TEMP
} else {
    [IO.Path]::GetTempPath()
}
$installRoot = Join-Path $temporaryRoot "rmd-slang-$version-$Platform"
$archivePath = Join-Path $temporaryRoot $archive.Name

if (Test-Path -LiteralPath $installRoot) {
    Remove-Item -Recurse -Force -LiteralPath $installRoot
}
New-Item -ItemType Directory -Path $installRoot | Out-Null

Write-Host "Downloading Slang $version for $Platform."
Invoke-WebRequest -Uri $downloadUrl -OutFile $archivePath

$actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
if ($actualHash -ne $archive.Sha256) {
    throw "Slang archive checksum mismatch: expected $($archive.Sha256), got $actualHash."
}

if ($archive.Name.EndsWith('.zip', [StringComparison]::Ordinal)) {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $installRoot
} else {
    & tar -xzf $archivePath -C $installRoot
    if ($LASTEXITCODE -ne 0) {
        throw "Could not extract '$archivePath'."
    }
}

$headerPath = Join-Path $installRoot 'include/slang.h'
$libraryName = if ($Platform -eq 'windows-x86_64') { 'slang.lib' } else { 'libslang.so' }
$libraryPath = Join-Path $installRoot "lib/$libraryName"
if (-not (Test-Path -LiteralPath $headerPath -PathType Leaf) -or
    -not (Test-Path -LiteralPath $libraryPath -PathType Leaf)) {
    throw "The Slang archive did not contain slang.h and $libraryName."
}

$binaryName = if ($Platform -eq 'windows-x86_64') { 'slang.dll' } else { 'slangc' }
$binaryPath = Join-Path $installRoot "bin/$binaryName"
if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
    throw "The Slang archive did not contain $binaryName."
}
$includeDirectory = Join-Path $installRoot 'include'
$libraryDirectory = Join-Path $installRoot 'lib'
$binaryDirectory = Join-Path $installRoot 'bin'

$variables = @{
    SLANG_INCLUDE_DIR = $includeDirectory
    SLANG_LIB_DIR = $libraryDirectory
}
if ($Platform -eq 'linux-x86_64') {
    $libraryPath = if ($env:LD_LIBRARY_PATH) {
        "$libraryDirectory$([IO.Path]::PathSeparator)$env:LD_LIBRARY_PATH"
    } else {
        $libraryDirectory
    }
    $variables.LD_LIBRARY_PATH = $libraryPath
}

foreach ($entry in $variables.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
    if ($env:GITHUB_ENV) {
        "$($entry.Key)=$($entry.Value)" >> $env:GITHUB_ENV
    }
}
if ($env:GITHUB_PATH) {
    $binaryDirectory >> $env:GITHUB_PATH
}

Remove-Item -Force -LiteralPath $archivePath
Write-Host "Configured Slang from '$installRoot'."
