# Changelog

`Bluestacks-Debloat` removes ads, tracking/telemetry, promotional spam and bloatware from BlueStacks 5,
with every change backed up and reversible. Releases are grouped by the BlueStacks version they target.

---

## v0.1.0 — initial release · BlueStacks 5.22+ · Android 9 / 11 / 13

First public release. Host-side + guest-side debloat in one file, with backup and undo.

### Added
- **One-file tool** (`blueStackDebloat.cmd` launcher + `blueStackDebloat.ps1` engine) that resolves your
  BlueStacks install/data paths and adb port from the registry + `bluestacks.conf` (same proven resolution
  as [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot)) — honours custom install locations.
- **Host-side debloat:** closes BlueStacks safely, backs up `bluestacks.conf` (UTF-8, no BOM), and disables
  ad / promotion / "click for rewards" / recommendation config keys (discovered by scanning the conf so it
  adapts across versions).
- **Guest-side debloat:** null-routes ad/analytics/telemetry domains in the guest `hosts` file, and disables
  preinstalled junk apps via `pm disable-user --user 0` (with `pm list packages` discovery + a curated
  bloat-pattern filter, so it adapts to what's actually installed).
- **Backups + Undo:** every change is saved to a timestamped backup folder; the **Undo** option restores
  `bluestacks.conf`, the guest `hosts` file, and re-enables disabled packages.
- **`--DryRun`/preview:** list exactly what *would* change before changing anything.

### Notes
- This initial release uses **runtime discovery** rather than hardcoded keys/package names so it adapts to
  your BlueStacks version. The curated bloat-match patterns in `blueStackDebloat.ps1` are a starting set —
  review and extend them for your setup (PRs/issues welcome).
- The guest-disk parts (hosts file, removing system apps) persist best alongside the version-proof
  HD-Player anti-tamper patch from [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot) on
  BlueStacks 5.22+, which otherwise reverts/kills tampered instances.
