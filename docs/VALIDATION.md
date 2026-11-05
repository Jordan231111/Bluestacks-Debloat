# Validation record

## 1.1.1 Android 13 live verification — 2026-11-05

This run used **BlueStacks 5.22.288.1014**, launcher **10.42.288.1001**, and the installed **Tiramisu64 / Android 13** instance. Successful changes were left applied, as requested; no restore/unroot cycle was performed in this run.

- The original UI applied Maximum's two remaining host changes and all four Android changes (disable `com.android.traceur` and set the three animation scales to `0.5`). Host ad flags and cloud cleanup were already configured, and the home-screen recommendation tiles remained until launcher isolation.
- A fresh root operation completed all six stages and verified Kitsune Magisk 31 with the temporary bootstrap removed.
- The original Network Apply reproduced a legacy HD-Adb protocol reset during a read-only root check. The updated build applied both Android hosts filtering and launcher isolation through the native UI successfully. Earlier activity logs showed the same transport faults during package/settings writes; those exact idempotent writes now use bounded retries with identity checks and read-back verification.
- **Two full stop/start cycles** independently reported zero host changes, zero Android changes, and no network-policy update needed. Magisk, the Android hosts overlay and both IPv4/IPv6 owner rules remained active. The selected instance kept 4 CPU cores and 8192 MiB RAM, high-FPS mode at 240 and VSync disabled. The game-ad tiles were absent after both restarts; the empty “Popular games to play” heading remained.
- Settings, Google Play and Chrome launched with Android's `Status: ok`. Screenshots showed the Play sign-in page and Chrome first-run page. Account sign-in, downloads and games were outside this check.
- The rolled-back, unsuccessful Network recovery point was deleted. The completed root/network recovery points passed integrity checks before their later deletion through the UI; automatic retention removed superseded points. Original system/player copies used for root recovery were retained.
- Pie64's displayed “7.3 GiB” was confirmed to be the sum of its live system/data disks and player executable, an estimate for a future backup. It remained visible with empty managed backup folders. The label now explicitly says “Estimated new backup size” and directs users to Backups & restore for actual saved points.
- The updated code passed formatting, Clippy with warnings denied and **103 tests**, including retry/ownership regressions, instance fallback, runtime UI refresh and 840 layout combinations.
- Fault injection terminated only the probe's private ADB daemon during a package write, an animation write and a root-network check. All three operations recovered and verified their original target values. A dependency audit reported zero known Rust dependency vulnerabilities and zero warnings.

Private logs, before/after screenshots and both restart inspections are under `.local/evidence/e2e-20261105` (excluded from Git). Earlier multi-version and restore validation below remains historical evidence, not a claim that every older Android variant or restore path was rerun for 1.1.1.

Local validation was performed on Windows 10 IoT Enterprise LTSC x64, BlueStacks **5.22.265.1013**. Version 1.1.0 adds disposable **Nougat32, Nougat64, Pie64, Rvc64 and Tiramisu64** tests to the original three-version validation below. The final build pins stable Rust **1.99.1**, edition 2024. This is a local validation record, not a claim of compatibility with every BlueStacks release or game.

## Updated toolchain and stress verification — 2026-11-04

All direct dependencies were checked against the crates.io stable releases and updated, including eframe/egui 0.37, sysinfo 0.40, winreg 0.56, SHA-2 0.11 and ZIP 9.3. The transitive lockfile was refreshed. The UI now uses eframe's root-UI/panel APIs, with clipboard and hyperlink support explicitly enabled. The new font stack removes the unmaintained ttf-parser dependency. The SHA-256 text representation and owned registry snapshots preserve existing recovery formats; both retained legacy recovery points passed their full integrity audit.

- **100 reconnect/package/animation inspections** passed across **five concurrent Android variants**. Median complete inspection was **3.052 seconds**, maximum **9.004 seconds** under this load.
- **Two complete five-variant cycles** applied every installed reviewed optional package and animation option, verified values after cold boots, reapplied without writes or extra recovery points, restored every recorded value, and verified the restoration after another cold boot. Wrong-identity refusal, an unavailable queue entry, grouped recovery retention and cancellation were checked as well.
- A **forced private-daemon exit** recovered with a new owned child and unchanged guest identity. Commands explicitly name the loopback server host to prevent adb from implicitly starting an untracked daemon. Transient setup faults restart only the owned foreground child.
- The fresh Nougat masters exposed a vendor **5555 port collision** when started together. Startup now reports an occupied configured port before launching. The concurrency runs used disposable clones with separate ports; the serial queue continues to support the masters individually.
- **840 automated layouts** exercised all seven tabs at four window sizes, five text scales and normal/error/busy states with 40 instance rows. No layout panic, invalid clip rectangle or unintended worker operation occurred.
- The native OpenGL window stayed responsive through **40 size changes**. A native UI fixture selected 12 instances, reviewed/applied 13 settings, reported zero further changes and restored the configuration byte-for-byte. Enlarged-text navigation was compacted to leave more room for content.
- Native root system-file preparation and read-back verification passed on an **expendable copied 8 GiB system partition**, including the staged commit. The original master was not written. A complete live root-install/unroot cycle was not repeated; the original root-cycle evidence below still describes its tested scope.
- Read-only root inspection passed for all four retained instances, including a clone whose system image is registered in its master machine. Root disk lookup now follows only current hardware image UUIDs through the shared machine registry. Regression cases reject unattached guesses and conflicting registrations, ignore snapshot-only images, and verify XML-escaped paths and shared-instance detection.

Stress reports are under `.local/evidence/stress-20261104` and excluded from Git. Temporary Android instances, copied disks, fixture files and stress executables are removed after verification; concise result records remain for the audit trail.

## 1.1.0 Android and multi-instance validation — 2026-11-03

Five fresh instances were created through BlueStacks' Multi-instance Manager, including downloading the two missing Nougat images. Android recovery and UI fixtures used isolated application-state directories. The original running Android 11 player remained open throughout guest testing.

| Check | Observed result |
|---|---|
| Reproduce the broken Android tab | The correct Android 11 port was refused because BlueStacks logged a stale `-0400` offset while Windows used `-0500`. Instance identification and package scanning passed after correction. |
| Idle player after shared-log rotation | Per-instance UTC startup headers retained the original player's identity after Player.log rotated. The native Android tab scanned its four reviewed optional packages and produced a review. |
| All five Android variants | Scan, review and apply succeeded on Nougat 32-bit, Nougat 64-bit, Android 9, Android 11 and Android 13. The final batch applied and verified 18 changes; package availability varied by version. |
| Persistence after cold boots | All five instances were closed and booted again. Every repeated review returned zero changes and no instance errors. This caught and corrected premature closure before delayed framework writes. Nougat uses a writeback hold and sync because its power command is blocked. |
| Exact recovery | Every batch recovery passed its integrity audit. Each instance was started, restored, closed and scanned again; recorded package and animation before/after values matched the original preview exactly. |
| Wrong Android identity | A deliberately mismatched identity was rejected before a transaction or package/settings mutation. |
| Partial batch | A deliberate unavailable entry between valid entries remained visible while all five real instances applied successfully. |
| Cancellation | Stopping after the first complete scan left the remaining four explicitly unscanned, with no temporary players left open. |
| Shared host transaction | A three-instance fixture verified one shared config write, deduplicated selection, unchanged CPU/RAM and unchecked-instance values, and exact restore. Empty and unknown selections were rejected. |
| Recovery retention | Five recovery copies from one batch survived as one recovery action. Regression tests also retired an entire old batch together. |
| Desktop workflow | Native UI checks covered explicit selection, Select all, Clear selection disabling Review, host review without writes, Android scanning/review, and fixed actions at 600×700 and 800×620 with 200% text. |
| Temporary-instance cleanup | All five test instances, including both newly downloaded Nougat masters, were deleted through the Manager. Only the original Pie64, Rvc64 and Tiramisu64 remain; their configuration keys match the pre-test copy. The original player PID and birth time were preserved. |

Private evidence is in `.local/evidence/batch-review`, including per-instance scans, previews, apply reports, backup audits, cold-boot results, exact-restore comparisons, cancellation and cleanup records. The full root installation/unroot cycle was not repeated for 1.1.0.

## Automated checks

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --all-targets`: **97 tests** (96 library tests and one 840-layout UI stress test).
- Optimized Windows x64 build with a static MSVC CRT.
- `cargo audit` (2026-11-04): **zero known Rust dependency vulnerabilities and zero warnings** after the complete dependency refresh. This audit does not cover the separate embedded root helper binaries.

Tests cover config byte preservation and stale previews, instance isolation, preserving CPU/RAM in Maximum, migrating away from legacy Google block entries, backup corruption (including same-length root configuration corruption), conflict-aware restore/retry, recovery despite journal-write failure, malformed/unsupported PE images, exact/idempotent binary edits, embedded payload checksums/extraction, partition bounds and short disk reads, and system-file ownership/mode verification.

Version 1.0.3 adds Windows read-only/hidden/system attribute, DACL and alternate-stream preservation tests; locked-file failure and rollback checks; deferred cloud-snapshot restore; and process ownership/exit-race checks. A native process fixture proves owned BstkSVC/HD-Adb helpers stop while a similarly named process in a sibling folder stays running. The built executable's embedded manifest is checked for `requireAdministrator`.

Feature-persistence tests verify the exact three-byte change, idempotence, composition with the existing integrity patch, and rejection of duplicate endpoints, missing executable references, overlay-only strings and unsupported binaries.

Additional cases cover exact hosts marker boundaries, duplicate rules and input domains, aliases, IPv4/IPv6 loopback blocks, conflicting manual mappings, comments and custom entries inside the managed section, UTF-8 BOM and legacy comment bytes, LF/CRLF, absent final newlines, unsupported encodings and damaged marker pairs. Six hundred generated hosts fixtures check preservation and repeated-edit stability.

Partial-apply tests prove that 22 independent changes survive one locked-file failure and all successful changes restore correctly. Other cases cover dependent-group rollback, stale config keys and external edits, unsupported future player patches, already-completed changes, absent hosts files, unavailable recovery storage, interrupted journal writes, missing checksums, oversized/unknown journal formats and injected recovery values.

## Live behavior checked

For 1.0.3, the native UI applied Maximum to Tiramisu64 while the player and BstkSVC were running. It synced Android, stopped both processes, verified shutdown, and applied all 22 pending changes (20 configuration edits, Windows hosts, and feature persistence). The actual hosts file had read-only, hidden and system attributes; its complete attribute value and access-control descriptor matched afterward, unrelated hosts entries remained intact, and CPU/RAM allocations matched for every instance. Tiramisu64 then booted and passed instance-specific Android inspection; a post-boot Maximum review reported zero pending operations. The persistence patch's recovery also restored the original player and flags during a separate controlled check. All three retained recovery points passed their full integrity audit.

The 1.0.1 regression tests cover retention across root/debloat and mixed time zones, protected unfinished records, truncated files, same-length file corruption, valid-JSON journal corruption, interrupted deletion, unexpected files, cloud archive ownership, restore preflight, operation-lock conflicts, and streaming copies with concurrent writers excluded. New journals carry checksums; old 1.0.0 journals remain compatible.

The updated Review → Apply → verified-result → no-changes flow was exercised through the native UI against an isolated configuration fixture. Review left the file unchanged; Apply changed the selected key, preserved CPU/RAM, created a checked recovery point, and displayed success. Live retention reduced 15 existing recovery points to the newest three, all of which passed a full integrity audit.

The fixed buttons remained visible at 600×700 and at 800×620 with 200% text size. A deliberately corrupted fixture backup triggered a visible warning, failed its integrity check, and disabled Restore; manually deleting that fixture backup left the active configuration untouched.

| Check | Observed result |
|---|---|
| Native UI from an isolated folder containing only the executable | Starts, detects the installed emulator and responds to navigation. |
| Desktop and compact layouts | Checked at wide and smaller windows, including 600×700 and 800×620; 150% and 200% text sizing reflows navigation/content with scrolling. Native Windows DPI handling remains enabled. |
| BlueStacks X / Services removal | Program folders, cloud profiles/updater caches, matching Run value, registrations, protocol and shortcuts removed from active locations with recovery copies. |
| Cloud removal → restore → removal | Completed, with exact read-back checks. Local player and Android data remained available. |
| Android 9/11/13 after cloud removal | All three reached Android and supported instance-specific ADB queries/screenshots. |
| Concurrent instance inspection | Three simultaneous package inspections used separate owned ADB servers and completed. |
| Wrong-instance ADB selection | Refused before guest changes. |
| Android ad-service disable and animation settings | Applied, verified, restored to their exact previous states, and reapplied. |
| Rooted network policy | Per-instance module installation, IPv4/IPv6 launcher rules and hosts overlay checked. Cold boots preserved rules/overlay; Google Play reached its sign-in page. |
| Network scope update | Owned modules migrated to BlueStacks-only domains. User/other hosts entries are preserved. |
| Integrity patch equivalence | Patching the original local player backup with Rust produced byte-for-byte the installed companion-patched player. One branch site changed. |
| Native VHD inspection | Actual VHD backup attached read-only, correct 8 GiB disk/ext4 region detected and detached. |
| Native offline system preparation | Verified on an expendable copied system disk before live root testing. |
| Complete native root repair on Android 11 | All six stages passed, including cold boot, Magisk root, original bindmount restoration, no competing `su`, and no exact bootstrap remnants. |
| Per-instance unroot on Android 11 | Removed Magisk/its modules/manager and verified root absent after reboot. |
| Root installation from that unrooted state | Complete native installation and verification passed. |
| Root-operation recovery | Both the new-root and unroot operation copies restored successfully; Magisk verification passed after restoring the rooted state. |
| Shared-system restoration | Removed root across the three instances, restored every original master and player, and verified all three restored Android versions booted without the Magisk daemon/manager. |
| Shared-system recovery | Restored the full pre-test recovery copy after the all-instance restore, returning the existing rooted system/data/player files. |

The initial filesystem and legacy ADB integration failures during development were detected and recovered. The final filesystem command behavior matches the companion's directory/relative-filename semantics; installer success is captured from both stdout and stderr.

## Scope limits

The complete root-installation cycles in the table were performed for the original Rust release. Version 1.0.1 rechecks the changed recovery/copy routines and UI through the regression suite and isolated fixtures; it does not repeat the invasive full root installation on each Android version.

The complete native installation/repair/unroot cycle was exercised on Android 11. Android 9 and 13 were exercised for boot, ADB, existing root verification, root network controls and shared-system unroot/restore. MSI App Player and unrelated BlueStacks releases need independent live validation.

The Google Play test stops at the sign-in screen. No user's account credentials, purchases, game saves or game-specific anti-cheat behavior were used for testing. No game FPS benchmark or universal “all traffic is blocked” claim is made. Network snapshots are process-owned TCP endpoints plus recent-log domains, not decrypted traffic inspection.

Local private logs and raw evidence are under `.local/evidence` and excluded from Git. Selected UI/Android screenshots in `docs/images` contain no account credentials. Root and debloat recovery copies remain in the application's local data directory.
