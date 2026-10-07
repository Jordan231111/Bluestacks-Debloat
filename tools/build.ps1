[CmdletBinding()]
param([switch]$SkipChecks)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $root
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path -LiteralPath $cargo)) { $cargo = (Get-Command cargo -ErrorAction Stop).Source }
function Invoke-Cargo([string[]]$Arguments) {
    & $cargo @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo failed: $($Arguments -join ' ')" }
}
if (-not $SkipChecks) {
    Invoke-Cargo @('fmt','--all','--','--check')
    Invoke-Cargo @('clippy','--locked','--all-targets','--','-D','warnings')
    Invoke-Cargo @('test','--locked','--all-targets')
}
Invoke-Cargo @('build','--release','--locked')
New-Item -ItemType Directory -Force -Path 'dist' | Out-Null
Copy-Item -LiteralPath 'target\x86_64-pc-windows-msvc\release\BluestacksDebloat.exe' -Destination 'dist\BluestacksDebloat.exe' -Force
Copy-Item -LiteralPath 'README.md','LICENSE','CHANGELOG.md' -Destination 'dist' -Force
Copy-Item -LiteralPath 'docs' -Destination 'dist' -Recurse -Force
New-Item -ItemType Directory -Force -Path 'dist\assets\root' | Out-Null
Copy-Item -LiteralPath 'assets\root\NOTICE.md','assets\root\bsr_su.c' -Destination 'dist\assets\root' -Force
Copy-Item -LiteralPath 'assets\licenses' -Destination 'dist\assets' -Recurse -Force
& (Join-Path $PSScriptRoot 'collect-licenses.ps1') -Cargo $cargo -OutputPath (Join-Path $root 'dist\THIRD-PARTY-NOTICES.txt')
$metadata = (Invoke-Cargo @('metadata','--locked','--no-deps','--format-version','1')) | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq 'bluestacks-debloat').version
$zipName = "BluestacksDebloat-$version-windows-x64.zip"
Compress-Archive -LiteralPath 'dist\BluestacksDebloat.exe','dist\README.md','dist\LICENSE','dist\CHANGELOG.md','dist\THIRD-PARTY-NOTICES.txt','dist\docs','dist\assets' -DestinationPath (Join-Path 'dist' $zipName) -Force
$hash = (Get-FileHash -LiteralPath 'dist\BluestacksDebloat.exe' -Algorithm SHA256).Hash.ToLowerInvariant()
$zipHash = (Get-FileHash -LiteralPath (Join-Path 'dist' $zipName) -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $root 'dist\SHA256SUMS.txt'), "$hash  BluestacksDebloat.exe`n$zipHash  $zipName`n", [Text.UTF8Encoding]::new($false))
Write-Host "Built: $root\dist\BluestacksDebloat.exe"
Write-Host "SHA-256: $hash"
