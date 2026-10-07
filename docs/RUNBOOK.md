# Run, verify, restore

## Normal use

Double-click `BluestacksDebloat.exe`. Use Overview to inspect or select custom folders. The program can inspect without administrator privileges; use its administrator restart button for host changes. Android tools need a running instance with local ADB enabled in BlueStacks Settings → Advanced.

For host changes, close all instances and the Multi-instance Manager. **Request normal close** asks BlueStacks to close; confirm BlueStacks' own exit dialog and allow shutdown to finish. The app refuses host writes while relevant processes remain. It does not forcibly kill running games.

Choose options, preview, and apply. Changing options or the selected instance invalidates the old preview. A stale underlying value also stops the operation. Successful operations are read back before being reported as verified.

After cloud removal, start the emulator using **Start selected instance**, an existing BlueStacks 5 shortcut, or `HD-Player.exe --instance NAME`. Test launch, your installed games and Google Play. The X/Store executable and the separate Services companion are not needed to invoke the local player directly.

## Backups

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
| Host process still running | Finish the BlueStacks exit dialog and close the Multi-instance Manager; wait for shutdown. |
| Cloud file in use | Close the X/Services application before previewing/applying removal. |
| Preview became stale | Refresh and preview again. |
| Optional patch has no validated signatures | Leave patching unchecked. Do not force offsets from another version. |
| Launcher search or Store fails | This is an expected effect of launcher network isolation. Use Google Play directly or restore the root-network operation. |
| Ads return after an update | Inspect and preview again; updates can replace flags, APKs and cloud products. |

Network reports and Android screenshots are saved locally under the app's data folder. They are not uploaded. Review them before sharing; screenshots can contain personal content.

For root installation, repair, unroot and full disk recovery, use the [rooting runbook](ROOTING.md). The Root tab performs shutdown, staged installation and verification in one operation. Its large disk recovery copies are distinct from ordinary debloat backups.

On a small laptop display, navigation moves to the top and controls stack vertically. Use the Text selector for 100–200% text sizing; scroll within the page for additional options. Windows DPI scaling remains active. Selecting **Maximum debloat** does not alter CPU/RAM allocations; those are explicitly manual controls.
