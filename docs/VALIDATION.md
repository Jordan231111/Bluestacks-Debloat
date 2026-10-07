# Validation record

Local validation was performed on Windows 10 IoT Enterprise LTSC x64, BlueStacks **5.22.265.1013**, with **Pie64, Rvc64 and Tiramisu64** instances. Rust is pinned to nightly **2026-10-07**, edition 2024. This is a local validation record, not a claim of compatibility with every BlueStacks release or game.

## Automated checks

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked --all-targets`: **39 tests**.
- Optimized Windows x64 build with a static MSVC CRT.
- `cargo audit` (2026-10-07): zero known Rust dependency vulnerabilities. One maintenance advisory remains for the UI's transitive `ttf-parser` 0.25.1 dependency (RUSTSEC-2026-0192); the app uses bundled fonts. This audit does not cover the separate embedded root helper binaries.

Tests cover config byte preservation and stale previews, instance isolation, preserving CPU/RAM in Maximum, migrating away from legacy Google block entries, backup corruption (including same-length root configuration corruption), conflict-aware restore/retry, recovery despite journal-write failure, malformed/unsupported PE images, exact/idempotent binary edits, embedded payload checksums/extraction, partition bounds and short disk reads, and system-file ownership/mode verification.

## Live behavior checked

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
