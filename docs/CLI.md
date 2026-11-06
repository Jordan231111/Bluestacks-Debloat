# Command line

Run `BluestacksDebloat.exe` without arguments for the desktop interface. Run `--help` for all commands. Previews are the default; changes require `--apply` or an explicit restore command.

Use an administrator PowerShell window for CLI commands. Double-clicking the app requests UAC automatically. Host apply/restore and root operations stop BlueStacks automatically; `close` performs the same verified shutdown without applying settings.

Preview commands return the plan and any unavailable options. Apply returns one JSON result with applied, unchanged, failed and skipped counts, issues and a recovery path. A partial result exits nonzero so scripts can detect it; successfully applied independent changes are retained. Review again to retry remaining options.

```powershell
.\BluestacksDebloat.exe scan
.\BluestacksDebloat.exe --instance Pie64 host --maximum
.\BluestacksDebloat.exe --instance Pie64 host --keep-features --apply
.\BluestacksDebloat.exe --instance Pie64 host --remove-x --remove-services --apply
.\BluestacksDebloat.exe --instance Pie64 packages
.\BluestacksDebloat.exe --instance Pie64 guest --recommended --animations --apply
.\BluestacksDebloat.exe --instances Pie64,Rvc64 host --high-fps
.\BluestacksDebloat.exe --instances Pie64,Rvc64 guest --recommended --animations --start-offline --apply
.\BluestacksDebloat.exe --all-instances host --maximum
.\BluestacksDebloat.exe --instance Pie64 root-network --hosts --isolate-launcher --apply
.\BluestacksDebloat.exe --instance Rvc64 root inspect
.\BluestacksDebloat.exe --instance Rvc64 root install --apply
.\BluestacksDebloat.exe --instance Rvc64 root verify
.\BluestacksDebloat.exe --instance Rvc64 root unroot --apply
.\BluestacksDebloat.exe root-backups
.\BluestacksDebloat.exe network
.\BluestacksDebloat.exe patch-info
.\BluestacksDebloat.exe --instance Pie64 screenshot .\android.png
.\BluestacksDebloat.exe backups
.\BluestacksDebloat.exe verify-backups
.\BluestacksDebloat.exe restore 'C:\Users\YOU\AppData\Local\BluestacksDebloat\backups\OPERATION'
```

`--install` and `--conf` override discovery. Specify `--instance` for instance-specific commands when several instances exist.

For `host`, `packages` and `guest`, use `--instances NAME,NAME` or `--all-instances` to select a batch. These flags cannot be combined with `--instance`. Host changes share one recovery transaction and apply common PC options once. Android batches return an `instances` array with a separate result/error for each instance; one unavailable instance does not block the others. An applied batch with any failures exits nonzero.

`guest --start-offline` boots stopped instances one at a time and closes only those players afterward. Already running instances remain open. Scanning saves exact package/settings values; Review uses that snapshot, and Apply verifies identity and stale values again. Without `--start-offline`, Android instances must already be running. Root, Network controls, Launch and Screenshot continue to require a single `--instance`.

Explicit package names outside the reviewed candidate list are reported as skipped. A reviewed app absent on another selected Android version is left unchanged there.

Completed recovery points are automatically limited to the newest three recovery actions after changes; every per-instance copy from one Android batch counts together. `cleanup-backups` runs that same fixed rule explicitly. `delete-backup 'C:\...\OPERATION'` permanently deletes one completed point; these two explicit cleanup commands do not use `--apply`. Unfinished and damaged recovery is reported and protected as described in the [runbook](RUNBOOK.md).

`host --maximum` selects the host stage, including BlueStacks-only hosts filtering and a high FPS limit. It preserves CPU/RAM and never silently roots Android. Apply guest and root-network changes separately while the selected instance is running.

`root install --repair --apply` performs the full repair pipeline even when root already works. `root full-unroot --apply` restores every original shared master and the shared player; read the [rooting guide](ROOTING.md) first.

For backup location, restore order, and troubleshooting, see the [runbook](RUNBOOK.md).

## KernelSU network backend

`root-network` accepts Magisk or a verified KernelSU runtime (built-in or LKM, UAPI 6 or newer) with an active Magic Mount-rs metamodule. Authorize Shell in the corresponding Manager first. `root install` remains the Magisk installer and is not a KernelSU installation command.
