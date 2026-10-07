# Root BlueStacks from the Rust app

Select **Android 9 / Pie64**, **Android 11 / Rvc64**, or **Android 13 / Tiramisu64**, open **Root BlueStacks**, and click **Root this instance** as administrator. Existing working Magisk root is detected. The repair option deliberately runs the full installation again.

Save game progress first. The workflow closes the installation's player/manager/VM helper processes, restarts the selected instance several times and creates recovery copies. CPU and RAM settings are preserved. The bundled APK, bootstrap and filesystem tools are checked against the current companion distribution's SHA-256 values before use.

## Native workflow

| Stage | Behavior |
|---|---|
| Preflight | Resolve actual `.bstk` disk references, require a supported instance, inspect VHD/VHDX content, check disk space and take the app/companion operation locks. |
| Recovery | Save and hash-check the system disk, selected Android data disk, player, config and machine file. A journal records progress before changes. |
| Preparation | Apply the guarded player patch if necessary, enable the temporary root/ADB flags, attach the VHD through Windows APIs, stage its ext4 partition and install the checked Magisk system files and bootstrap. |
| Installation | Boot the correct instance, verify ADB ownership/identity, install the manager and populate `/data/adb/magisk`. Set the per-instance root flag. |
| Initialization | Reboot, confirm the Magisk daemon and grant local shell UID 2000 access through Magisk's policy database. |
| Cleanup | Remove temporary bootstrap `su`, restore the stock bindmount script and verify system file bytes, ownership and permissions. |
| Finalization | Turn emulator root off, retain shareable readonly system/boot disk modes and leave the instance's Android data writable. |
| Verification | Cold-boot, verify Magisk root and its `su` link, inspect standard `su` locations and sweep for the exact bootstrap hash. |

Host orchestration and disk access are Rust. No PowerShell or `.cmd` root engine is invoked. The exact guest init/shell templates and open-source helper binaries are embedded; they are extracted into a temporary workspace automatically. The original `BluestacksRoot` checkout is not needed by the compiled executable.

## Shared disks and unroot

Clones may use the same `Root.vhd`. The companion's gate script is preserved: Magisk starts only for an instance whose own `/data` contains the root flag. Rooting one instance does not create that flag in other instances.

**Unroot this instance** removes its Magisk flag, framework data/modules and manager, then reboots and checks the result. Other instances retain their root. This also removes root-dependent debloat filtering in the selected instance.

**Restore all original shared root files** is an advanced operation affecting every installed Android version. It requires the original player backup and a system backup for every unique master. It removes per-instance Magisk data, restores every master together, restores the player and verifies the restored instances. Restoring one master while unpatching a player shared with other modified masters is avoided.

## Recovery

Root journals and disk copies live under `%LOCALAPPDATA%\BluestacksDebloat\root-backups`. The Root tab lists them separately from smaller debloat backups. Restore newer root operations first.

A root recovery copy includes the full selected Android data disk. Restoring it returns that instance's apps/data to the saved point in time. Config recovery targets root/ADB keys while preserving unrelated configuration. Applying and restoring root changes require the relevant instances to be closed; the root workflow handles shutdown itself.

Failed root operations attempt recovery automatically. A failure before committing the staged system partition leaves the source system disk unchanged. Backup hashes are checked before recovery; incomplete recovery remains visible in the journal and is not reported as success.

## CLI

```powershell
.\BluestacksDebloat.exe --instance Rvc64 root inspect
.\BluestacksDebloat.exe --instance Rvc64 root install --apply
.\BluestacksDebloat.exe --instance Rvc64 root install --repair --apply
.\BluestacksDebloat.exe --instance Rvc64 root verify
.\BluestacksDebloat.exe --instance Rvc64 root unroot --apply
.\BluestacksDebloat.exe --instance Rvc64 root full-unroot --apply
.\BluestacksDebloat.exe root-backups
.\BluestacksDebloat.exe root-restore 'C:\Users\YOU\AppData\Local\BluestacksDebloat\root-backups\OPERATION'
```

Root/install/unroot commands without `--apply` show information only. Verification can start the selected instance. In PowerShell automation, pipe GUI-subsystem executable output to `Out-String` or use `Start-Process -Wait -PassThru` when waiting for its exit code.

The corresponding component sources and checksums are described in [payload provenance](../assets/root/NOTICE.md). The implementation follows the companion's current supported Magisk workflow rather than its archived legacy rooting scripts.
