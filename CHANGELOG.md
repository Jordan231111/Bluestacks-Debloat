# Changelog

## 1.1.0 — 2026-11-03

- Move to stable Rust 1.99.1 and update all direct dependencies and the transitive lockfile; the dependency audit has no known vulnerabilities or warnings.
- Upgrade the UI to eframe/egui 0.37, preserve clipboard/link support, remove the unmaintained font parser, and compact navigation at enlarged text sizes.
- Recover failed private ADB daemons without borrowing or implicitly starting an untracked server; recheck player ownership before reconnecting.
- Resolve cloned instances' shared root disks by their registered image UUIDs, preserving private data-disk selection and rejecting ambiguous or snapshot-only bindings.
- Report configured ADB port conflicts before starting an instance, and skip booting already-clean entries during mixed batch Apply.
- Fix Android tools rejecting the correct instance after daylight-saving changes or shared-log rotation; retain process ownership and stale-PID checks.
- Retry interrupted read-only ADB commands, including transient Android 13 transport failures.
- Check both supported BlueStacks launcher packages, including older Android images.
- Select multiple instances with explicit checkboxes, Select all, Clear selection and instance search.
- Apply host settings for the selection in one configuration transaction; apply shared PC changes once.
- Scan and debloat Android instances in a queue, starting and closing stopped instances individually while keeping existing players open.
- Preserve delayed Android package/settings writes before closing queued players, including Nougat's writeback window.
- Review the exact Android scan snapshot without another boot; show per-instance results and allow stopping after the current instance.
- Keep every instance's recovery copy from a batch together under the newest-three-actions retention policy.
- Add `--instances`, `--all-instances` and `guest --start-offline` to the CLI.

## 1.0.3 — 2026-10-07

- Keep successful independent changes when another option fails, with applied/failed/skipped counts and recovery.
- Harden hosts editing against duplicates, conflicting mappings, damaged markers and encoding problems while preserving custom entries.
- Automatically stop BlueStacks players, managers, ADB and companion services before host changes, restore and rooting.
- Request Windows administrator access when the app opens.
- Fix Maximum debloat failing on read-only Windows hosts files; preserve permissions and attributes, and retry brief file locks.
- Keep Maximum's feature choices across restarts with a reversible, validated player patch that prevents remote configuration refresh.
- Allow reviewing cloud removal while the cloud app is running; capture its recovery files after automatic shutdown.

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
