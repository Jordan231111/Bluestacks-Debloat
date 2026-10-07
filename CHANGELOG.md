# Changelog

## 1.0.2 — 2026-10-07

- Clear a previous root verification result when rooting, unrooting, restoring, or checking an operation that fails.
- Stop automatic root-support retries after an inspection error so the problem stays readable.
- Enforce the backup limit after a failed apply has been fully rolled back as well.

Includes the visible Apply bar, automatic three-backup retention, and integrity checks from 1.0.1.

## 1.0.1 — 2026-10-07

- Keep Review and Apply visible in a fixed bottom bar, with pending counts and verified results.
- Automatically retain the newest three verified recovery points across debloat and root; add manual deletion and integrity checks.
- Surface damaged and incomplete backups, protect unresolved recovery, and resume interrupted deletion.
- Check every recovery file before restoring; checksum new journals and hash large disks during copying.

## 1.0.0 — 2026-10-07

- Rebuilt in Rust as a standalone Windows x64 executable, with an adaptive desktop UI and CLI.
- Added separate BlueStacks X/Store and cloud Services removal with recovery copies.
- Ported the companion Magisk workflow: install, repair, verify, unroot, and restore disks.
- Added Maximum debloat while preserving CPU/RAM, plus BlueStacks-only network controls.
- Added isolated ADB connections, exact instance checks, previews, verified backups, and conflict-aware recovery.

Validated on BlueStacks 5.22.265.1013. See [tested behavior and limits](docs/VALIDATION.md).

## 0.1.0 — historical script release

- Initial CMD launcher and PowerShell engine.
- Host configuration changes, Android package/hosts changes, and backup/undo.

The Rust release replaces this implementation and its broader network rules.
