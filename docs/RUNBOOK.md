# Runbook — debloat, verify, and roll back

## Before you start
- BlueStacks 5 (nxt) or MSI App Player installed and **launched at least once** (so `bluestacks.conf` exists).
- Run as **Administrator** (editing `bluestacks.conf` and stopping BlueStacks services needs it).

## Debloat
1. **Fully close** BlueStacks, including the Multi-Instance Manager and the system-tray icon.
2. Right-click **`blueStackDebloat.cmd` → Run as administrator**.
3. (Optional) choose **Preview / Dry-run** first — it prints exactly which config keys, domains and packages
   it would change, without changing anything.
4. Choose **Full Debloat**, or pick individual categories (ads, telemetry/hosts, App Center, junk apps).
5. Re-launch BlueStacks.

## Verify it worked
- **Ads:** boot the instance — no start-up ad; no in-app banners during normal use.
- **Telemetry:** ad/analytics domains resolve to `0.0.0.0` inside the guest
  (`adb shell cat /system/etc/hosts`).
- **Apps:** the App Center / recommendation apps no longer launch
  (`adb shell pm list packages -d` shows them disabled).
- **Speed:** idle RAM/CPU is lower with the helper processes gone.

## Roll back (Undo)
Run the tool again and choose **Undo**. It will:
1. Restore the backed-up `bluestacks.conf`.
2. Restore the guest `hosts` file.
3. Re-enable any packages it disabled (`pm enable`).

Backups live in a timestamped `backups\` folder next to the script. To restore manually, copy the saved
`bluestacks.conf` back over the live one (keep it **UTF-8 without BOM**) while BlueStacks is closed.

## Troubleshooting
- **"adb device not found":** BlueStacks writes its real adb port on boot. Launch the instance once, then
  re-run; the tool re-reads `status.adb_port` from `bluestacks.conf`.
- **Ads returned after an update:** updates can rewrite `bluestacks.conf`. Just re-run the tool.
- **Changes reverted / instance closed itself on 5.22+:** apply the HD-Player anti-tamper patch from
  [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot) so guest-disk edits stick, then re-run.
