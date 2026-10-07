# Command line

Run `BluestacksDebloat.exe` without arguments for the desktop interface. Run `--help` for all commands. Previews are the default; changes require `--apply` or an explicit restore command.

```powershell
.\BluestacksDebloat.exe scan
.\BluestacksDebloat.exe --instance Pie64 host --maximum
.\BluestacksDebloat.exe --instance Pie64 host --remove-x --remove-services --apply
.\BluestacksDebloat.exe --instance Pie64 packages
.\BluestacksDebloat.exe --instance Pie64 guest --recommended --animations --apply
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

Completed recovery points are automatically limited to the newest three after changes. `cleanup-backups` runs that same fixed rule explicitly. `delete-backup 'C:\...\OPERATION'` permanently deletes one completed point; these two explicit cleanup commands do not use `--apply`. Unfinished and damaged recovery is reported and protected as described in the [runbook](RUNBOOK.md).

`host --maximum` selects the host stage, including BlueStacks-only hosts filtering and a high FPS limit. It preserves CPU/RAM and never silently roots Android. Apply guest and root-network changes separately while the selected instance is running.

`root install --repair --apply` performs the full repair pipeline even when root already works. `root full-unroot --apply` restores every original shared master and the shared player; read the [rooting guide](ROOTING.md) first.

For backup location, restore order, and troubleshooting, see the [runbook](RUNBOOK.md).
