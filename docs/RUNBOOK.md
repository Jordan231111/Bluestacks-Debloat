# Run, verify, restore

## Normal use

Double-click `BluestacksDebloat.exe` and accept the Windows UAC prompt. Use Overview to inspect or select custom folders. Administrator access is requested automatically when the app opens. Android tools need a running instance with local ADB enabled in BlueStacks Settings → Advanced.

Applying or restoring host changes automatically stops the selected installation's players, Multi-instance Manager, ADB, BstkSVC and cloud companions. Root operations use the same shutdown routine. The app attempts to sync reachable Android instances, requests normal closure, then terminates remaining verified BlueStacks processes and waits for shutdown before writing. Save progress in running games before applying. Review alone does not stop anything. **Stop BlueStacks** in Overview also runs this automatic shutdown.

Read-only hosts/config files are handled automatically: the app temporarily clears the read-only attribute for replacement, preserves the file's access permissions and other attributes, and restores the original protection afterward. Brief file-sharing conflicts are retried. A failing independent option is reported and its attempted change is recovered; other successful options remain applied.

Choose options, review, and apply. Changing options or the selected instance invalidates the old review. Stale target values are preserved and reported; independent valid changes continue. Successful writes are read back before being reported as verified. The result shows applied, already-set, failed and skipped counts, with errors in Activity. Review again to retry only the remaining changes.

Cloud removal keeps its files and registrations together: a failure restores changes in that group and skips the rest of it. Root installation, repair and unroot remain ordered workflows and stop when a required stage fails. Failure to establish ownership, acquire an operation lock or maintain durable recovery can stop dependent work; the app does not write without a usable recovery record.

Hosts editing recognizes exact standalone marker lines, reuses equivalent existing IPv4/IPv6 blocks, and preserves custom entries and comments. It retains existing text bytes, BOM and line endings. Conflicting manual mappings, damaged marker pairs and unsupported encodings are reported without changing the hosts file; other independent options can still apply. Each hostname normally needs one IPv4 rule and one IPv6 rule.

BlueStacks normally refreshes feature flags from its server during startup. **Keep these choices after restart**, included in Maximum, uses a validated player patch to stop that refresh. It affects all instances; configuration remains writable for normal settings and runtime ports. A recovery copy restores the original player and flags. Basic choices without this option may be reset by BlueStacks.

**Review** and **Apply** stay in the fixed bottom bar on Debloat, BlueStacks X, Android apps, and Network. Review does not apply anything. The bar shows pending changes, no changes needed, or a verified completion message. Checkboxes select the next operation; they are not indicators of the installed state.

After cloud removal, start the emulator using **Start selected instance**, an existing BlueStacks 5 shortcut, or `HD-Player.exe --instance NAME`. Test launch, your installed games and Google Play. The X/Store executable and the separate Services companion are not needed to invoke the local player directly.

## Backups

The app automatically keeps the **newest three verified completed recovery points total**, across normal and root operations. This runs on desktop startup and after changes. Before removing older points, it checks the retained recovery files. Unfinished, unrecognized, or damaged recovery records are reported and protected when they need inspection; these can temporarily exceed the three-point limit. There is no retention setting.

Use **Delete backup** to permanently remove a completed point and its recorded cloud recovery copies. **Check backup integrity** streams each file through SHA-256 and shows missing, truncated, or corrupt content. All recovery files are checked before an explicit restore changes live files. New journals contain their own checksum; v1.0.0 journals remain readable, but their original metadata did not have that checksum.

Cleanup records its exact file list before deletion and resumes an interrupted deletion. Unknown files, links/junctions, unexpected archive paths, and modified cloud recovery folders stop cleanup and remain visible for inspection. Cleanup failures are reported separately from successful configuration changes.

Journals, file backups and UI logs live under `%LOCALAPPDATA%\BluestacksDebloat`. Each operation has a timestamp and UUID, so backups do not overwrite one another. Directory recovery copies live beside their original cloud folder under `.BlueStacksDebloat-backups` to preserve same-volume rename semantics. The journal records their paths.

Use **Backups & restore** and select the exact operation. Restore recent overlapping operations in reverse order. Config restoration edits only the recorded keys, preserving newer runtime ports/counters. Whole binary/file and registry restores refuse to overwrite newer differing content. A conflict leaves its recovery data available and reports `restore_incomplete`; resolve the reported conflict before retrying.

If power is lost, an `applying` journal records the operation that may have been interrupted. Restore checks both the saved before and after states and can resume repeatedly. Do not edit journals or move/delete recovery folders while you need rollback.

Android backups include the exact instance and a hashed Android identity. Start that same instance before restoring. An instance recreated under the same name is rejected if its identity differs. Root-network restore removes the owned firewall chain immediately; reboot Android to remove an active hosts overlay. It does not unroot the instance or remove another Magisk module.

## Troubleshooting

| Message / symptom | Action |
|---|---|
| Missing or ambiguous installation | Set both Program and Data folders in Overview. Program must contain HD-Player/HD-Adb; Data must contain bluestacks.conf. |
| Instance offline | Start the selected instance and wait for its home screen. |
| ADB port belongs to another instance | Refresh after boot. Do not assume every configured `5555` belongs to the selected instance. |
| Root denied / timed out | Grant the normal Magisk shell request in that instance, then retry; basic debloat remains available without root. |
| Shutdown cannot be verified | No offline changes start. The error identifies the process/service that Windows could not stop. |
| Cloud file in use after automatic shutdown | Retry after the reported lock is released; completed changes are rolled back and recovery is retained. |
| Preview became stale | Refresh and preview again. |
| Optional patch has no validated signatures | Leave patching unchecked. Do not force offsets from another version. |
| Launcher search or Store fails | This is an expected effect of launcher network isolation. Use Google Play directly or restore the root-network operation. |
| Ads return after an update | Inspect and preview again; updates can replace flags, APKs and cloud products. |

Network reports and Android screenshots are saved locally under the app's data folder. They are not uploaded. Review them before sharing; screenshots can contain personal content.

For root installation, repair, unroot and full disk recovery, use the [rooting runbook](ROOTING.md). The Root tab performs shutdown, staged installation and verification in one operation. Its large disk recovery copies are distinct from ordinary debloat backups.

On a small laptop display, navigation moves to the top and controls stack vertically. Use the Text selector for 100–200% text sizing; scroll within the page for additional options. Windows DPI scaling remains active. Selecting **Maximum debloat** does not alter CPU/RAM allocations; those are explicitly manual controls.
