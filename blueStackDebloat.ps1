<#
  Bluestacks-Debloat  -  remove ads, tracking/telemetry, promo spam and bloatware from BlueStacks 5.
  Open source (CC BY-NC-ND 4.0).  https://github.com/Jordan231111/Bluestacks-Debloat

  v0.1.0 - initial release.  Host-side (bluestacks.conf, processes) + guest-side (hosts file, packages).
  Every change is backed up first and is reversible via -Undo.

  Path / data-dir / adb-port resolution mirrors the proven approach in the companion BluestacksRoot project
  (registry: BlueStacks_nxt then BlueStacks_msi5, native + WOW6432Node; conf: status.adb_port then adb_port).

  USAGE (run as Administrator):
    powershell -ExecutionPolicy Bypass -File blueStackDebloat.ps1            # interactive menu
    ... -DryRun                                                             # preview only, change nothing
    ... -Full                                                               # debloat everything, no menu
    ... -Undo                                                               # restore from the latest backup
#>
[CmdletBinding()]
param(
    [switch]$Full,
    [switch]$Undo,
    [switch]$DryRun,
    [string]$Install,                       # BlueStacks install dir (auto-detected if omitted)
    [string]$Conf                           # path to bluestacks.conf (auto-detected if omitted)
)

$ErrorActionPreference = 'Stop'
function Say($m, $c = 'Gray') { Write-Host $m -ForegroundColor $c }

# --- curated, EDITABLE discovery patterns ------------------------------------------------------------
# Conf keys are only touched if they match this and currently hold a boolean-ish flag (0/1/true/false).
$AdConfKeyRegex   = '(?i)(show_ads|enable_ads|(^|\.)ads?(_|$)|ad_unit|promot|campaign|recommend|reward|offer|banner|app_install)'
# Guest packages matched here become candidates to disable (shown for confirmation first).
$BloatPkgRegex    = '(?i)(bluestacks.*(promo|appcenter|appfinder|center|helper|store|hint|launcherhelper)|com\.bsl\.|gameloft|com\.android\.egg)'
# Ad / analytics / telemetry domains null-routed in the guest hosts file. Conservative by design.
$BlockDomains = @(
    'googleads.g.doubleclick.net','pagead2.googlesyndication.com','googlesyndication.com',
    'www.googleadservices.com','adservice.google.com','app-measurement.com',
    'ads.bluestacks.com','cloudslivessmedia.bluestacks.com','adsdk.bluestacks.com'
)
# -----------------------------------------------------------------------------------------------------

# 1) Registry: InstallDir / DataDir / UserDefinedDir (nxt then msi5; native + WOW6432Node)
function Get-BstReg {
    foreach ($k in @('HKLM:\SOFTWARE\BlueStacks_nxt','HKLM:\SOFTWARE\BlueStacks_msi5',
                     'HKLM:\SOFTWARE\WOW6432Node\BlueStacks_nxt','HKLM:\SOFTWARE\WOW6432Node\BlueStacks_msi5')) {
        try {
            $p = Get-ItemProperty -LiteralPath $k -ErrorAction Stop
            if ($p -and ($p.InstallDir -or $p.DataDir -or $p.UserDefinedDir)) {
                return [pscustomobject]@{ InstallDir = $p.InstallDir; DataDir = $p.DataDir; UserDefinedDir = $p.UserDefinedDir }
            }
        } catch {}
    }
    return $null
}

# 2) Normalize to the folder that actually holds bluestacks.conf (newer builds report DataDir as ...\Engine)
function Get-BaseDir($reg) {
    $cands = New-Object System.Collections.Generic.List[string]
    foreach ($d in @($reg.DataDir, $reg.UserDefinedDir)) {
        if ($d) {
            if ($d -match '(?i)[\\/]engine[\\/]?$') { [void]$cands.Add(($d -replace '(?i)[\\/]engine[\\/]?$', '')) }
            [void]$cands.Add($d)
        }
    }
    [void]$cands.Add((Join-Path $env:ProgramData 'BlueStacks_nxt'))
    foreach ($c in $cands) { if ($c -and (Test-Path -LiteralPath (Join-Path $c 'bluestacks.conf'))) { return $c } }
    return $cands[0]
}

$reg = Get-BstReg
if (-not $Install) { $Install = if ($reg -and $reg.InstallDir) { $reg.InstallDir.TrimEnd('\') } else { Join-Path $env:ProgramFiles 'BlueStacks_nxt' } }
if (-not $Conf)    { $Conf = Join-Path (Get-BaseDir $reg) 'bluestacks.conf' }
$Adb = Join-Path $Install 'HD-Adb.exe'

$BackupRoot = Join-Path $PSScriptRoot 'backups'
$LatestLink = Join-Path $BackupRoot 'latest'

Say "BlueStacks install : $Install"  Cyan
Say "bluestacks.conf    : $Conf"     Cyan
if (-not (Test-Path -LiteralPath $Conf)) { Say "[!] bluestacks.conf not found - launch BlueStacks once, then re-run." Red; return }

# --- helpers ----------------------------------------------------------------------------------------
function Stop-BlueStacks {
    Say '[*] Closing BlueStacks (HD-*, Bstk*, BlueStacks*)...' Yellow
    Get-Process | Where-Object { $_.ProcessName -match '^(HD-|Bstk|BlueStacks)' } |
        ForEach-Object { try { $_.Kill() } catch {} }
    Start-Sleep -Seconds 2
}

function Read-ConfLines { [System.IO.File]::ReadAllLines($Conf) }
function Write-ConfLines($lines) {
    # BlueStacks requires UTF-8 WITHOUT BOM; preserve LF/CRLF style of the original.
    $nl = if ((Get-Content -Raw -LiteralPath $Conf) -match "`r`n") { "`r`n" } else { "`n" }
    $enc = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Conf, ($lines -join $nl) + $nl, $enc)
}

function New-Backup {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $dir = Join-Path $BackupRoot $stamp
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    Copy-Item -LiteralPath $Conf -Destination (Join-Path $dir 'bluestacks.conf') -Force
    if (Test-Path $LatestLink) { Remove-Item $LatestLink -Recurse -Force }
    Copy-Item -LiteralPath $dir -Destination $LatestLink -Recurse -Force
    Say "[+] Backup saved: $dir" Green
    return $dir
}

# adb port: status.adb_port (runtime) then adb_port; verify it's a BlueStacks instance
function Get-AdbPort {
    foreach ($key in @('status\.adb_port', 'adb_port')) {
        $m = Select-String -LiteralPath $Conf -Pattern "^bst\.[^=]*$key=`"?(\d+)" -ErrorAction SilentlyContinue |
             Select-Object -First 1
        if ($m) { return [int]$m.Matches[0].Groups[1].Value }
    }
    return 5555
}
function Invoke-Adb { param([Parameter(ValueFromRemainingArguments)]$a) & $Adb @a 2>&1 }
function Connect-Adb {
    if (-not (Test-Path $Adb)) { Say "[!] HD-Adb.exe not found at $Adb" Red; return $null }
    $port = Get-AdbPort
    Invoke-Adb 'kill-server' | Out-Null
    Invoke-Adb 'connect' "127.0.0.1:$port" | Out-Null
    Start-Sleep -Seconds 1
    return "127.0.0.1:$port"
}

# --- host-side: disable ad/promo conf keys ----------------------------------------------------------
function Invoke-HostDebloat {
    $lines = Read-ConfLines
    $hits = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^([^=]+)="?([^"]*)"?\s*$') {
            $k = $Matches[1]; $v = $Matches[2]
            if ($k -match $AdConfKeyRegex -and $k -notmatch '(?i)adb|update|download|thread|read|load' -and $v -match '^(0|1|true|false)$') {
                $hits += [pscustomobject]@{ Index = $i; Key = $k; Value = $v }
            }
        }
    }
    if (-not $hits) { Say '[*] No ad/promo config keys found to change.' DarkGray; return }
    Say "[*] Ad/promo config keys found:" Cyan
    $hits | ForEach-Object { Say ("    {0} = {1}  ->  0/false" -f $_.Key, $_.Value) }
    if ($DryRun) { Say '[dry-run] (no changes made)' Yellow; return }
    foreach ($h in $hits) {
        $off = if ($h.Value -match '^(true|false)$') { 'false' } else { '0' }
        $lines[$h.Index] = ($lines[$h.Index] -replace [regex]::Escape('"' + $h.Value + '"'), ('"' + $off + '"'))
        if ($lines[$h.Index] -match ('=' + [regex]::Escape($h.Value) + '\s*$')) { $lines[$h.Index] = $lines[$h.Index] -replace ([regex]::Escape('=' + $h.Value) + '\s*$'), ('=' + $off) }
    }
    Write-ConfLines $lines
    Say "[+] Disabled $($hits.Count) ad/promo config key(s) (UTF-8 no BOM)." Green
}

# --- guest-side: hosts file + package disable -------------------------------------------------------
function Invoke-GuestDebloat {
    $dev = Connect-Adb
    if (-not $dev) { return }
    Invoke-Adb '-s' $dev 'root' | Out-Null; Start-Sleep 1
    Invoke-Adb '-s' $dev 'remount' | Out-Null

    # hosts file
    $existing = (Invoke-Adb '-s' $dev 'shell' 'cat /system/etc/hosts') -join "`n"
    Set-Content -LiteralPath (Join-Path $LatestLink 'hosts.bak') -Value $existing -ErrorAction SilentlyContinue
    $toAdd = $BlockDomains | Where-Object { $existing -notmatch [regex]::Escape($_) }
    if ($toAdd) {
        Say "[*] Block domains (guest hosts): $($toAdd -join ', ')" Cyan
        if (-not $DryRun) {
            foreach ($d in $toAdd) { Invoke-Adb '-s' $dev 'shell' "echo '0.0.0.0 $d' >> /system/etc/hosts" | Out-Null }
            Say "[+] Null-routed $($toAdd.Count) ad/telemetry domain(s)." Green
        } else { Say '[dry-run] (hosts unchanged)' Yellow }
    } else { Say '[*] Ad/telemetry domains already blocked.' DarkGray }

    # packages
    $pkgs = (Invoke-Adb '-s' $dev 'shell' 'pm list packages') -split "`n" |
            ForEach-Object { ($_ -replace '^package:', '').Trim() } | Where-Object { $_ }
    $cands = $pkgs | Where-Object { $_ -match $BloatPkgRegex }
    if ($cands) {
        Say "[*] Bloat package candidates to disable:" Cyan
        $cands | ForEach-Object { Say "    $_" }
        $disabled = @()
        if (-not $DryRun) {
            foreach ($p in $cands) { Invoke-Adb '-s' $dev 'shell' "pm disable-user --user 0 $p" | Out-Null; $disabled += $p }
            $disabled -join "`n" | Set-Content -LiteralPath (Join-Path $LatestLink 'disabled-packages.txt')
            Say "[+] Disabled $($disabled.Count) bloat package(s) (reversible via Undo)." Green
        } else { Say '[dry-run] (packages unchanged)' Yellow }
    } else { Say '[*] No bloat packages matched.' DarkGray }
}

# --- undo -------------------------------------------------------------------------------------------
function Invoke-Undo {
    if (-not (Test-Path $LatestLink)) { Say '[!] No backup found to restore.' Red; return }
    Stop-BlueStacks
    $confBak = Join-Path $LatestLink 'bluestacks.conf'
    if (Test-Path $confBak) { Copy-Item -LiteralPath $confBak -Destination $Conf -Force; Say '[+] Restored bluestacks.conf' Green }
    $dev = Connect-Adb
    if ($dev) {
        Invoke-Adb '-s' $dev 'root' | Out-Null; Invoke-Adb '-s' $dev 'remount' | Out-Null
        $hostsBak = Join-Path $LatestLink 'hosts.bak'
        if (Test-Path $hostsBak) { Invoke-Adb '-s' $dev 'push' $hostsBak '/system/etc/hosts' | Out-Null; Say '[+] Restored guest hosts' Green }
        $pkgList = Join-Path $LatestLink 'disabled-packages.txt'
        if (Test-Path $pkgList) { Get-Content $pkgList | Where-Object { $_ } | ForEach-Object { Invoke-Adb '-s' $dev 'shell' "pm enable $_" | Out-Null }; Say '[+] Re-enabled disabled packages' Green }
    }
    Say '[+] Undo complete.' Green
}

# --- entry ------------------------------------------------------------------------------------------
if ($Undo) { Invoke-Undo; return }

function Invoke-All { Stop-BlueStacks; if (-not $DryRun) { New-Backup | Out-Null }; Invoke-HostDebloat; Invoke-GuestDebloat; Say '[+] Done. Re-launch BlueStacks.' Green }

if ($Full) { Invoke-All; return }

# interactive menu
while ($true) {
    Say ''
    Say '==== Bluestacks-Debloat ====' Magenta
    Say '  1) Full debloat (ads + telemetry + bloat apps)'
    Say '  2) Host-side only (disable ad/promo config)'
    Say '  3) Guest-side only (hosts + bloat apps)'
    Say '  4) Preview / dry-run (change nothing)'
    Say '  5) Undo (restore latest backup)'
    Say '  0) Exit'
    $c = Read-Host 'Choose'
    switch ($c) {
        '1' { Invoke-All }
        '2' { Stop-BlueStacks; if (-not $DryRun) { New-Backup | Out-Null }; Invoke-HostDebloat }
        '3' { Stop-BlueStacks; if (-not $DryRun) { New-Backup | Out-Null }; Invoke-GuestDebloat }
        '4' { $script:DryRun = $true; Stop-BlueStacks; Invoke-HostDebloat; Invoke-GuestDebloat; $script:DryRun = $false }
        '5' { Invoke-Undo }
        '0' { return }
        default { Say 'Pick 0-5.' Yellow }
    }
}
