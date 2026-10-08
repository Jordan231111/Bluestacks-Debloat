# Architecture and research evidence

## Runtime

The application is Rust 2024 with a native egui/eframe desktop window and an optional Clap command line. There is no browser, localhost UI server, Electron, Python, Java or PowerShell runtime in the application. The executable uses Windows registry/file/process/TCP APIs and BlueStacks' existing HD-Adb for guest commands. The MSVC CRT is statically linked; Windows system DLLs and the graphics driver remain normal OS dependencies.

Slow discovery, hashing, ADB and changes run on a worker thread. The UI streams progress, disables competing operations and prevents ordinary window close while work is in progress. Operation locks prevent competing mutations in the same app state directory.

## Operation model

`discovery` → `engine` → reviewable `Plan` → `transaction` journal → mutation → read-back verification.

Host apply/restore and root operations share `shutdown`, ported from the companion's `Kill-BlueStacks` flow. It checks executable paths and process birth times, handles BstkSVC/ADB/manager/cloud helpers, stops matching Windows application services through the service manager, and verifies closure. Cloud directory fingerprints are captured after shutdown so runtime log/database changes do not invalidate a removal preview. Kernel driver startup configuration is preserved.

The embedded manifest requests administrator privileges on launch. Atomic file replacement temporarily clears read-only protection, restores original file attributes on success or failure, and retains destination DACLs and alternate streams through `ReplaceFileW`. Sharing conflicts have a bounded retry; access permissions are not broadened.

Maximum also selects a reversible player patch for the remote configuration-refresh endpoint, preventing boot-time replacement of local feature flags. It checks PE section bounds, a unique terminated endpoint and an executable RIP-relative reference, and composes with the existing integrity patch in one backed-up file operation. The endpoint replacement has equal length and changes three bytes; the full player is available for exact recovery. Other cloud endpoints and the writable configuration file remain available.

The transaction prepares before/after recovery before its first mutation and journals each entry before writing it. Independent operations continue after a local failure; successful changes remain applied. Dependent cloud-removal groups roll back their own completed entries on failure. The report and journal retain failed/skipped details and incomplete recovery. Config uses targeted reverse edits; binary/files/registry use checked snapshots. Directory removal is a hashed, same-volume move. Global recovery-storage failures stop further writes and preserve resumable progress.

Hosts parsing is byte-preserving and recognizes only complete marker lines. Existing aliases are compared case-insensitively per address family; custom content is retained and conflicting mappings are rejected. New journals carry a format marker that makes a missing checksum an error, with a bounded metadata size and compatibility for legacy journals. Atomic replacement retains an original-file fallback for Windows partial-rename failures and rechecks expected contents before replacing a target.

ADB connection selection requires a current instance config, a live TCP listener owned by the corresponding HD-Player, and current process/log evidence. Protected player command lines are handled by timestamp/PID correlation against Player.log. The selected Android identity is checked across preview/apply/restore, and before package/setting/root mutations. Private foreground ADB servers use 15037–15057, each with an exclusive port lock and an owned child-process handle. Shutdown disconnects the selected device and stops only that owned child. The application never kills a global ADB server on 5037 or borrows a companion tool's daemon.

## Companion-project review

The local `BluestacksRoot` repository was inspected in detail: `tools/bsr_host.ps1`, `tools/bsr_engine.ps1`, resolver/process/ADB helpers, `docs/BLUESTACKS_ROOTING_DEEP_DIVE.md`, and the patch flow. The Rust implementation incorporates the useful invariants:

- Registry InstallDir / DataDir / UserDefinedDir records and native/WOW64 views.
- DataDir can point into `Engine`; locate the folder actually containing `bluestacks.conf`.
- Clone/instance names are explicit; runtime ADB ports are preferred over configured defaults.
- Shared Root.vhd files are not appropriate targets for per-instance runtime debloat.
- HD-Player can hide its executable/command line from ordinary WMI queries; process birth time prevents stale log PIDs from matching a new process.
- Private ADB server ownership avoids disrupting another Android session.
- The integrity patch has primary string anchors and pre-existing NOP detection.

The old debloat script stopped the emulator before guest operations, stopped processes in dry-run mode, ignored ADB failure status, matched packages broadly, and copied incomplete backup state into a mutable `latest` directory. Those execution paths were replaced rather than wrapped.

The full Magisk host workflow is now ported in `rooting.rs`, `virtual_disk.rs`, `root_files.rs` and `root_assets.rs`. Windows VHD APIs replace PowerShell storage cmdlets. MBR/GPT bounds, GPT checksums and the ext4 marker identify the staging region. Every written system file is checked for bytes, owner, mode and links before committing. Commit scans the staging image and writes only changed blocks. Full data-disk recovery copies extend the companion's backup coverage.

The filesystem tool requires its directory/relative-filename command semantics; this was verified against an expendable copied VHD. Legacy HD-Adb can report installer success on stderr, so APK installation/uninstallation capture both streams and require explicit completion. Binary screenshots and shell output remain separate. Shell commands use the non-PTY `exec-out` path with an exit marker.

The UI uses logical points with native DPI handling, compact navigation on narrow windows, stacked controls, wrapped actions and scrollable content. The text-size preference is independent of Windows DPI. Instance/path/text preferences survive normal close and administrator restart. Maximum selects high FPS and debloat controls while keeping the CPU/RAM allocation preset unchanged; this invariant is tested.

## Local inspection

Inspected BlueStacks 5.22.265.1013 with Pie64, Rvc64 and Tiramisu64 entries. Existing config provided the exact rule names. The launcher package was inspected locally to understand the recommendation path: `/app_player/get_suggested_apps`, promotion/statistics calls and the failure behavior that hides the recommendation area. This supported a narrowly scoped launcher UID filter rather than blocking the shared hostnames that also serve required APIs. No decompiled vendor source/APK is redistributed.

Live Android enumeration distinguished `gg.now.ads.service` from launcher, billing, Google and account packages. The local X folder existed despite missing X uninstall registration, and Services had a separate HKCU uninstall record and Run entry. Removal therefore uses verified product markers and exact registrations, and keeps the App Player installation independent.

Network inspection is a connection/log snapshot, not TLS interception or a claim to classify all encrypted traffic. Endpoint associations are not inferred from IP address alone.

## Primary references

- [BlueStacks' documented gameplay ad preference](https://support.bluestacks.com/hc/en-us/articles/10721739047309-How-to-disable-ads-in-BlueStacks-5).
- [Android ADB documentation](https://developer.android.com/tools/adb), including package and shell operations.
- [Magisk module developer guide](https://topjohnwu.github.io/Magisk/guides.html), for system overlays and service scripts.
- [Rust Windows MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
- [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot), for project-specific installation, ADB and disk architecture evidence.
