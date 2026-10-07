[CmdletBinding()]
param([Parameter(Mandatory)][string]$Cargo, [Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$raw = & $Cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Could not resolve dependency licenses.' }
$metadata = $raw | ConvertFrom-Json
$activeIds = @($metadata.resolve.nodes | ForEach-Object { $_.id })
$fallbacks = @{
    'clipboard-win' = @('clipboard-win-BSL.txt')
    'ecolor' = @('egui-MIT.txt', 'egui-Apache.txt')
    'eframe' = @('egui-MIT.txt', 'egui-Apache.txt')
    'egui' = @('egui-MIT.txt', 'egui-Apache.txt')
    'egui-winit' = @('egui-MIT.txt', 'egui-Apache.txt')
    'egui_glow' = @('egui-MIT.txt', 'egui-Apache.txt')
    'emath' = @('egui-MIT.txt', 'egui-Apache.txt')
    'epaint' = @('egui-MIT.txt', 'egui-Apache.txt')
    'epaint_default_fonts' = @('egui-MIT.txt', 'egui-Apache.txt')
    'gl_generator' = @('gl_generator-Apache.txt')
    'khronos_api' = @('khronos_api-Apache.txt')
    'profiling' = @('profiling-MIT.txt', 'profiling-Apache.txt')
}
$text = [Text.StringBuilder]::new()
[void]$text.AppendLine('BlueStacks Debloat: third-party notices')
[void]$text.AppendLine('Includes resolved Windows, build, and test dependencies. Some entries are not linked into the executable.')
[void]$text.AppendLine('Root helper attribution and source links are in assets/root/NOTICE.md; additional texts are in assets/licenses/.')
foreach ($package in $metadata.packages | Where-Object { $_.id -in $activeIds -and $_.name -ne 'bluestacks-debloat' } | Sort-Object name,version) {
    [void]$text.AppendLine("`n============================================================")
    [void]$text.AppendLine("$($package.name) $($package.version) -- $($package.license)")
    [void]$text.AppendLine("Source: $($package.repository)")
    $directory = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $directory -File -Recurse | Where-Object {
        $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|OFL|UFL)([-._]|$)' -or
        ($package.name -eq 'epaint_default_fonts' -and $_.Extension -eq '.txt')
    } | Sort-Object FullName)
    foreach ($file in $files) {
        $relative = $file.FullName.Substring($directory.Length).TrimStart('\','/').Replace('\','/')
        [void]$text.AppendLine("`n--- $relative ---")
        [void]$text.AppendLine([IO.File]::ReadAllText($file.FullName))
    }
    if ($files.Count -eq 0 -or $package.name -eq 'epaint_default_fonts') {
        if (-not $fallbacks.ContainsKey($package.name)) { throw "Missing license text for $($package.name)." }
        foreach ($name in $fallbacks[$package.name]) {
            [void]$text.AppendLine("`n--- upstream $name ---")
            [void]$text.AppendLine([IO.File]::ReadAllText((Join-Path $root "assets\licenses\$name")))
        }
    }
}
[IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), $text.ToString(), [Text.UTF8Encoding]::new($false))
