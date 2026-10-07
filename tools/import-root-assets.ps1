[CmdletBinding()]
param([string]$Source = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'BluestacksRoot'))
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root 'assets\root'
New-Item -ItemType Directory -Path $dest -Force | Out-Null
$cmd = [IO.File]::ReadAllText((Join-Path $Source 'blueStackRoot.cmd'))
$expected = @{
    APK = 'fac319d2de262fcfff1684e13e1a5c61c486d2a773a7a8ffcfdbfe6f763a7fd4'
    DFS = '008b6006e766d2591c8c7db7bf6d6a0a4b9cd6116b9a8e2737151828eb577632'
}
foreach ($entry in @(@('APK','magisk.apk'),@('DFS','debugfs.zip'),@('BSRSU','bootstrap.gz'))) {
    $tag=$entry[0]; $match=[regex]::Match($cmd,"(?ms)^__BSR_${tag}_BEGIN__\r?\n(.*?)^__BSR_${tag}_END__")
    if(-not $match.Success){throw "Missing embedded $tag"}
    $bytes=[Convert]::FromBase64String($match.Groups[1].Value)
    $path=Join-Path $dest $entry[1]
    [IO.File]::WriteAllBytes($path,$bytes)
    $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if($expected.ContainsKey($tag) -and $hash -ne $expected[$tag]){throw "$tag payload hash mismatch"}
    Write-Host "$($entry[1]): $($bytes.Length) bytes; SHA-256 $hash"
}
$script = [IO.File]::ReadAllText((Join-Path $Source 'tools\bsr_magisk.ps1'))
foreach($item in @(@('BOOTANIM_RC','bootanim.rc'),@('BSR_BOOT_SH','bsr_boot.sh'),@('BINDMOUNT_MOD','bindmount.bootstrap'),@('BINDMOUNT_ORIG','bindmount.stock'))) {
    $pattern='(?ms)^\$'+$item[0]+'\s*=\s*@''\r?\n(.*?)^''@'
    $match=[regex]::Match($script,$pattern)
    if(-not $match.Success){throw "Missing template $($item[0])"}
    # A PowerShell here-string excludes the final newline before its closing token.
    $text=$match.Groups[1].Value.Replace("`r`n","`n")
    if($text.EndsWith("`n")){$text=$text.Substring(0,$text.Length-1)}
    [IO.File]::WriteAllText((Join-Path $dest $item[1]),$text,[Text.UTF8Encoding]::new($false))
}
Copy-Item -LiteralPath (Join-Path $Source 'tools\su_src\bsr_su.c') -Destination (Join-Path $dest 'bsr_su.c') -Force
Write-Host 'Imported binary assets and guest templates. No PowerShell runtime is embedded.'
