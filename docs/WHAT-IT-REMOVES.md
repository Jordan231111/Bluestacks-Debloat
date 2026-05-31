# What `Bluestacks-Debloat` removes

Every item below is **backed up before** it changes and can be restored with **Undo**. The tool *discovers*
the exact keys/packages on your machine at run time (by scanning `bluestacks.conf` and `pm list packages`),
so this list describes the **categories** it targets rather than a fixed set that drifts between versions.

## Host-side (your Windows install — `bluestacks.conf`, processes)

| Target | What it is | Effect of removing |
|---|---|---|
| In-app ads | Banner/overlay ads shown during use | No more in-app ad banners |
| Start-up ad | The ad shown when an instance boots | Faster, ad-free boot |
| "Click for rewards" / promos | Reward nags and promoted-app popups | No upsell nags |
| Promotional notifications | "Tips", offers, app suggestions | Quiet system tray |
| Helper / agent processes | Background processes beyond the engine | Lower idle RAM/CPU |

> The tool scans `bluestacks.conf` for keys matching ad / promo / campaign / recommendation patterns,
> shows you what it found, and sets them to disabled (writing the file back as UTF-8 **without** a BOM and
> preserving its formatting, which BlueStacks requires).

## Guest-side (inside Android — `hosts`, packages)

| Target | How | Effect |
|---|---|---|
| Ad / analytics / telemetry domains | Null-routed in the guest `/system/etc/hosts` | Ads & tracking can't phone home |
| App Center / recommendation apps | `pm disable-user --user 0 <pkg>` | No game-recommendation wall |
| Preinstalled junk APKs | `pm disable-user` / `pm uninstall --user 0` | Reclaimed space, less clutter |

> Guest packages are matched against a curated bloat-pattern list (e.g. BlueStacks helper/promo packages and
> common preinstalled junk). The tool lists candidates and asks before disabling. Disabling (rather than
> hard-uninstalling) is the default because it is always reversible.

## What it does **not** touch
- It never modifies or redistributes any BlueStacks binary you can't restore.
- It doesn't touch your games, Google account, or saved data.
- It doesn't change anything outside your BlueStacks install/data folders + the guest instance.

## Persistence on BlueStacks 5.22+
Host-side changes (`bluestacks.conf`, processes) persist on their own. The guest-disk changes (hosts file,
system-app removal) persist best when the instance's disk-integrity check is patched — see the version-proof
HD-Player patch in [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot). Without it, BlueStacks
5.22+ may revert guest-disk edits or terminate the instance for "tampering."
