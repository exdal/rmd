[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $ByondBin,

    [Parameter(Mandatory = $true)]
    [string] $OutputPath,

    [Parameter(Mandatory = $true)]
    [string] $ExpectedVersion
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([Environment]::Is64BitProcess) {
    throw 'BYOND is a 32-bit application. Run this script with 32-bit Windows PowerShell.'
}

$ByondBin = [IO.Path]::GetFullPath($ByondBin)
$corePath = Join-Path $ByondBin 'byondcore.dll'

if (-not (Test-Path -LiteralPath $corePath -PathType Leaf)) {
    throw "Could not find byondcore.dll in '$ByondBin'."
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class ByondNative
{
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool SetDllDirectory(string path);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibrary(string path);

    [DllImport("kernel32.dll", CharSet = CharSet.Ansi, SetLastError = true)]
    public static extern IntPtr GetProcAddress(IntPtr module, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool FreeLibrary(IntPtr module);

    // StdDefDM is exported as a member function, but its implementation does
    // not access an instance. Calling the zero-argument export is the same
    // mechanism Dream Maker uses to assemble the generated file.
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    public delegate IntPtr StdDefDelegate();
}
'@

if (-not [ByondNative]::SetDllDirectory($ByondBin)) {
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    throw "SetDllDirectory failed with Windows error $errorCode."
}

$module = [IntPtr]::Zero

try {
    $module = [ByondNative]::LoadLibrary($corePath)
    if ($module -eq [IntPtr]::Zero) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Loading byondcore.dll failed with Windows error $errorCode."
    }

    $exportName = '?StdDefDM@DungBuilder@@QAEPADXZ'
    $procedure = [ByondNative]::GetProcAddress($module, $exportName)
    if ($procedure -eq [IntPtr]::Zero) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "The Dream Maker StdDefDM export was not found (Windows error $errorCode)."
    }

    $delegateType = [type] [ByondNative+StdDefDelegate]
    $generator = [Runtime.InteropServices.Marshal]::GetDelegateForFunctionPointer(
        $procedure,
        $delegateType
    )
    $generatedPointer = $generator.Invoke()

    if ($generatedPointer -eq [IntPtr]::Zero) {
        throw 'Dream Maker returned an empty pointer for stddef.dm.'
    }

    $generated = [Runtime.InteropServices.Marshal]::PtrToStringAnsi($generatedPointer)
}
finally {
    if ($module -ne [IntPtr]::Zero) {
        [ByondNative]::FreeLibrary($module) | Out-Null
    }
    [ByondNative]::SetDllDirectory($null) | Out-Null
}

if ([string]::IsNullOrWhiteSpace($generated)) {
    throw 'Dream Maker returned no stddef.dm content.'
}

$versionPattern = '(?m)^\s*Version\s+' + [regex]::Escape($ExpectedVersion) + '\s*$'
if ($generated -notmatch '(?m)^\s*Standard definitions for Dream Maker\s*$' -or
    $generated -notmatch $versionPattern) {
    throw "The generated header does not identify BYOND $ExpectedVersion."
}

$bodyMarker = '// directions'
$bodyStart = $generated.IndexOf($bodyMarker, [StringComparison]::Ordinal)
if ($bodyStart -lt 0) {
    throw "The generated stddef.dm does not contain the '$bodyMarker' marker."
}

$fileContent = $generated
$fileContent = $fileContent.Replace("`r`n", "`n").Replace("`r", "`n")
$fileContent = $fileContent.TrimEnd([char[]] "`r`n") + "`n"

if ($fileContent.Length -lt 1000 -or
    -not $fileContent.Contains('#define NORTH 1') -or
    -not $fileContent.Contains('var/const')) {
    throw 'The generated stddef.dm failed its content checks.'
}

$OutputPath = [IO.Path]::GetFullPath($OutputPath)
[IO.File]::WriteAllText($OutputPath, $fileContent, (New-Object Text.UTF8Encoding($false)))

Write-Host "Extracted stddef.dm for BYOND $ExpectedVersion to '$OutputPath'."
