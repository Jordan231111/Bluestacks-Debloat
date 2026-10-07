# Scope of the Rust edition

The executable's rule lists are in `src/rules.rs` (host settings), `src/adb.rs` (Android packages), `src/network.rs` (domains), and `src/cloud.rs` (cloud products). The preview is authoritative for the installed build.

## Default host choices

Disable known gameplay/boot ad controls, programmatic ad features, ad/statistics event controls and Smart Downloads. Only existing settings with recognized values are edited. The tool preserves config ordering, quotes, whitespace, line endings and unrelated content, and refuses duplicate keys.

Optional cloud choices disable X integration, cloud-instance UI, nowBux/rewards, cloud recording uploads, AI integrations and related known flags. Notification and performance settings apply to the chosen instance; global flags affect all instances.

Optional manual CPU/RAM presets reserve host resources, offering up to 4 cores/4 GiB for Balanced, 8 cores/8 GiB for Gaming, or 2 cores/2 GiB for Low Memory. **Maximum keeps existing CPU and RAM allocations.** Its independent high-FPS choice sets a 240 FPS limit, enables high-frame-rate mode and disables emulator VSync; achievable FPS remains game/hardware-dependent. The renderer and resolution stay as configured.

## Cloud removal

Verified X/Store and Services installation folders, their specifically named user profiles/updater caches, matching Run values, exact product registrations, matching protocol commands and shortcuts that reference those actual cloud installation paths. Each folder is moved to a recovery location on the same volume; every file is hashed before the move and verified afterwards. Reparse points are refused.

The App Player's install/data folders, `.bstk` files, virtual disks, VM services/drivers and Android games are excluded. Custom scheduled tasks and existing Windows firewall rules are not removed. This app does not call the vendor's combined uninstall/cleanup utility.

## Android packages

Candidates include `gg.now.ads.service`, `com.uncube.gamevantage`, legacy BlueStacks App Center/finder IDs, and separately selectable Android Easter egg, Print Spooler and system-tracing UI. Only actually installed candidates are shown. Optional printing removal disables printing. Packages are disabled for Android user 0; their data is retained and their original enabled state is recorded exactly.

Launcher, Google Play, Google Services, billing, account components, keyboard and arbitrary games are never selected by regex or publisher name.

## Optional root network module

Magisk module ID: `bluestacks_debloat`, stored in `/data/adb/modules/bluestacks_debloat` within the selected instance. Its blocklist contains only `ads.bluestacks.com` and `adsdk.bluestacks.com`. Google and game hostnames are excluded. Updating an older managed block removes obsolete entries inside that owned block while preserving unrelated hosts content. This filtering feature uses no writable system remount or shared system-disk edit; the separate Root tab performs system-disk changes when installing Magisk.

Launcher isolation owns only the `BSD_LAUNCHER_V1` iptables/ip6tables chains. It derives the UID for `com.uncube.launcher3`, rejects privileged/shared UIDs, preserves loopback and the local host bridge, and rejects other traffic from that UID. The script recalculates the UID at boot. Unrelated firewall chains remain intact. Launcher search, remote recommendations and online Store functions are unavailable while isolated; installed apps can still launch.

## Optional binary patch

Primary integrity strings plus nearby RIP-relative references identify CALL/TEST/JZ sequences inside the x64 `.text` section. PE bounds, call targets and branch destinations are checked. Only validated two-byte branch sites change. A complete backup is saved before an atomic replacement. There is no unanchored force mode and no broad fallback scan. Updates may require a new implementation; unsupported executables are refused.
