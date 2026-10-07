//! Removes cloud products independently of the local Android engine. Vendor
//! all-product cleaners are deliberately not invoked: their scope includes VMs.
use crate::{
    discovery::Installation,
    platform,
    transaction::{self, Operation, RegistryPath, Target, Value},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use winreg::{RegKey, enums::*};

pub fn cloud_roots(remove_x: bool, remove_services: bool) -> Vec<(PathBuf, &'static str)> {
    let mut result = Vec::new();
    if remove_x {
        for env in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(env) {
                let dir = PathBuf::from(p).join("BlueStacks X");
                if dir.join("BlueStacks X.exe").is_file() {
                    result.push((dir, "BlueStacks X.exe"));
                }
            }
        }
    }
    if remove_services && let Some(p) = std::env::var_os("LOCALAPPDATA") {
        let dir = PathBuf::from(p).join("Programs/bluestacks-services");
        if dir.join("BlueStacksServices.exe").is_file() {
            result.push((dir, "BlueStacksServices.exe"));
        }
    }
    // Custom cloud installs: only exact cloud display names and a verified marker.
    for machine in [false, true] {
        for view32 in [false, true] {
            if !machine && view32 {
                continue;
            }
            let h = RegKey::predef(if machine {
                HKEY_LOCAL_MACHINE
            } else {
                HKEY_CURRENT_USER
            });
            if let Ok(uninstall) = h.open_subkey_with_flags(
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                KEY_READ
                    | if view32 {
                        KEY_WOW64_32KEY
                    } else {
                        KEY_WOW64_64KEY
                    },
            ) {
                for name in uninstall.enum_keys().flatten() {
                    if let Ok(k) = uninstall.open_subkey(&name) {
                        let display: String = k.get_value("DisplayName").unwrap_or_default();
                        let marker = if remove_x
                            && matches!(display.as_str(), "BlueStacks X" | "BlueStacks Store")
                        {
                            Some("BlueStacks X.exe")
                        } else if remove_services && display == "BlueStacks Services" {
                            Some("BlueStacksServices.exe")
                        } else {
                            None
                        };
                        if let Some(marker) = marker {
                            let cmd: String = k.get_value("UninstallString").unwrap_or_default();
                            let exe = cmd
                                .strip_prefix('"')
                                .and_then(|v| v.split_once('"'))
                                .map(|v| v.0);
                            if let Some(dir) = exe.and_then(|e| Path::new(e).parent())
                                && dir.join(marker).is_file()
                            {
                                result.push((dir.into(), marker));
                            }
                        }
                    }
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    result.retain(|(p, _)| {
        seen.insert(
            platform::absolute(p)
                .unwrap_or_else(|_| p.clone())
                .to_string_lossy()
                .to_lowercase()
                .replace('/', "\\"),
        )
    });
    result
}
fn cloud_display(display: &str, x: bool, services: bool) -> bool {
    (x && matches!(display, "BlueStacks X" | "BlueStacks Store"))
        || (services && matches!(display, "BlueStacks Services" | "BlueStacksServices"))
}
fn references(text: &str, roots: &[(PathBuf, &str)]) -> bool {
    let text = text.to_ascii_lowercase().replace('/', "\\");
    roots
        .iter()
        .any(|(p, _)| text.contains(&p.to_string_lossy().to_ascii_lowercase().replace('/', "\\")))
}
fn shortcut_references(bytes: &[u8], roots: &[(PathBuf, &str)]) -> bool {
    // Shell links contain the full local target as ANSI and/or UTF-16LE. Require
    // the actual cloud root in the link; the shortcut's display name is insufficient.
    let ansi = String::from_utf8_lossy(bytes);
    let wide: String = String::from_utf16_lossy(
        &bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    );
    let shifted: String = String::from_utf16_lossy(
        &bytes
            .get(1..)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    );
    references(&ansi, roots) || references(&wide, roots) || references(&shifted, roots)
}
pub fn add_removal(
    install: &Installation,
    x: bool,
    services: bool,
    operations: &mut Vec<Operation>,
    notes: &mut Vec<String>,
) -> Result<()> {
    let roots = cloud_roots(x, services);
    let id = uuid::Uuid::new_v4().to_string();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for (root, marker) in &roots {
        let root = platform::absolute(root)?;
        ensure!(
            !platform::same_path(&root, &install.install_dir)
                && !root.starts_with(&install.data_dir)
                && !install.data_dir.starts_with(&root)
                && !root.join("HD-Player.exe").exists()
                && !root.join("bluestacks.conf").exists(),
            "Cloud removal target overlaps emulator files"
        );
        for p in sys.processes().values() {
            ensure!(
                !p.exe().is_some_and(|exe| exe.starts_with(&root)),
                "Close {} before previewing cloud removal",
                p.name().to_string_lossy()
            );
        }
        ensure!(
            root.join(marker).is_file(),
            "Cloud product marker disappeared"
        );
        let stash = root
            .parent()
            .context("Cloud path has no parent")?
            .join(".BlueStacksDebloat-backups")
            .join(&id)
            .join(root.file_name().unwrap());
        // Both paths are absolute and the archive is a sibling on the same drive.
        ensure!(
            !stash.exists() && stash.is_absolute() && !stash.starts_with(&root),
            "Invalid removal backup path"
        );
        operations.push(Operation {
            label: format!(
                "Remove {} from its active location (keep recovery copy)",
                root.display()
            ),
            target: Target::Directory {
                fingerprint: transaction::tree_hash(&root)?,
                path: root,
                stash,
            },
            before: Value::Present(true),
            after: Value::Present(false),
        });
    }
    // Cloud-only user profiles and updater caches; never the emulator's data root.
    for (enabled, variable, folder) in [
        (x, "LOCALAPPDATA", "BlueStacks X"),
        (services, "APPDATA", "bluestacks-services"),
        (services, "LOCALAPPDATA", "bluestacks-services-updater"),
    ] {
        if !enabled {
            continue;
        }
        let Some(base) = std::env::var_os(variable) else {
            continue;
        };
        let path = PathBuf::from(base).join(folder);
        if !path.is_dir() {
            continue;
        }
        let path = platform::absolute(&path)?;
        ensure!(
            !path.join("HD-Player.exe").exists()
                && !path.join("Engine").exists()
                && !path.join("bluestacks.conf").exists(),
            "Cloud cache overlaps emulator data"
        );
        let stash = path
            .parent()
            .context("Missing cloud profile parent")?
            .join(".BlueStacksDebloat-backups")
            .join(&id)
            .join(path.file_name().unwrap());
        operations.push(Operation {
            label: format!(
                "Remove cloud profile/cache: {} (keep recovery copy)",
                path.display()
            ),
            target: Target::Directory {
                fingerprint: transaction::tree_hash(&path)?,
                path,
                stash,
            },
            before: Value::Present(true),
            after: Value::Present(false),
        });
    }
    // Keep Windows registrations and their precise types so restoration is lossless.
    let mut seen = BTreeSet::new();
    for machine in [false, true] {
        for view32 in [false, true] {
            if !machine && view32 {
                continue;
            }
            let h = RegKey::predef(if machine {
                HKEY_LOCAL_MACHINE
            } else {
                HKEY_CURRENT_USER
            });
            let flags = KEY_READ
                | if view32 {
                    KEY_WOW64_32KEY
                } else {
                    KEY_WOW64_64KEY
                };
            let run = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
            if let Ok(k) = h.open_subkey_with_flags(run, flags) {
                for item in k.enum_values() {
                    let (name, value) = item?;
                    if matches!(value.vtype, REG_SZ | REG_EXPAND_SZ) {
                        let text = String::from_utf16_lossy(
                            &value
                                .bytes
                                .as_chunks::<2>()
                                .0
                                .iter()
                                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                                .collect::<Vec<_>>(),
                        );
                        if references(&text, &roots) {
                            let p = RegistryPath {
                                machine,
                                view32,
                                key: run.into(),
                                name: name.clone(),
                            };
                            let before = transaction::read_registry(&p)?;
                            operations.push(Operation {
                                label: format!("Remove cloud autostart: {name}"),
                                target: Target::Registry(p),
                                before,
                                after: Value::Missing,
                            });
                        }
                    }
                }
            }
            let mut keys = Vec::new();
            if x {
                keys.extend([
                    r"SOFTWARE\BlueStacks X".to_owned(),
                    r"SOFTWARE\BlueStacksX".into(),
                ]);
            }
            if services {
                keys.push(r"SOFTWARE\BlueStacksServices".into());
            }
            let uninstall = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
            if let Ok(k) = h.open_subkey_with_flags(uninstall, flags) {
                for name in k.enum_keys().flatten() {
                    if let Ok(v) = k.open_subkey(&name) {
                        let display: String = v.get_value("DisplayName").unwrap_or_default();
                        if cloud_display(&display, x, services) {
                            keys.push(format!("{uninstall}\\{name}"));
                        }
                    }
                }
            }
            let classes = r"SOFTWARE\Classes";
            if !view32 && let Ok(k) = h.open_subkey_with_flags(classes, flags) {
                for name in k.enum_keys().flatten().filter(|n| {
                    let s = n.to_ascii_lowercase();
                    s.starts_with("bsx") || s.starts_with("bluestacks") || s.starts_with("nowgg")
                }) {
                    if let Ok(cmd) = k.open_subkey(format!("{name}\\shell\\open\\command")) {
                        let value: String = cmd.get_value("").unwrap_or_default();
                        if references(&value, &roots) {
                            keys.push(format!("{classes}\\{name}"));
                        }
                    }
                }
            }
            for key in keys {
                let before = transaction::read_tree(machine, view32, &key)?;
                if before != Value::Missing && seen.insert((machine, view32, key.clone())) {
                    operations.push(Operation {
                        label: format!("Remove cloud registration: {key}"),
                        target: Target::RegistryTree {
                            machine,
                            view32,
                            key,
                        },
                        before,
                        after: Value::Missing,
                    });
                }
            }
        }
    }
    let mut folders = Vec::new();
    for (env, sub) in [
        ("APPDATA", r"Microsoft\Windows\Start Menu\Programs"),
        ("ProgramData", r"Microsoft\Windows\Start Menu\Programs"),
        ("PUBLIC", "Desktop"),
        ("USERPROFILE", "Desktop"),
    ] {
        if let Some(p) = std::env::var_os(env) {
            folders.push(PathBuf::from(p).join(sub));
        }
    }
    for dir in folders {
        if !dir.is_dir() {
            continue;
        }
        let mut stack = vec![(dir, 0)];
        while let Some((dir, depth)) = stack.pop() {
            for e in fs::read_dir(&dir)?.flatten() {
                let p = e.path();
                let ft = e.file_type()?;
                if ft.is_dir()
                    && depth == 0
                    && e.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with("bluestacks")
                {
                    stack.push((p, depth + 1));
                } else if ft.is_file()
                    && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                {
                    let bytes = fs::read(&p)?;
                    if shortcut_references(&bytes, &roots) {
                        operations.push(Operation {
                            label: format!("Remove cloud shortcut: {}", p.display()),
                            target: Target::File { path: p },
                            before: Value::Bytes(bytes),
                            after: Value::Missing,
                        });
                    }
                }
            }
        }
    }
    notes.push("BlueStacks X / Services removal preserves the local App Player, Multi-instance Manager, Android disks and games. Cloud app folders move to a recovery directory on the same drive, and related registrations/shortcuts are backed up. This initially saves background activity, not disk space; recovery copies occupy their original size. Updates can reinstall cloud components.".into());
    if roots.is_empty() {
        notes.push("No active cloud product folder found. Any matching leftover cloud registrations are still included.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_player_never_matches_cloud_product_name() {
        for n in [
            "BlueStacks",
            "BlueStacks 5",
            "MSI App Player",
            "BlueStacks_nxt",
        ] {
            assert!(!cloud_display(n, true, true));
        }
        assert!(cloud_display("BlueStacks X", true, false));
        assert!(!cloud_display("BlueStacks Services", true, false));
    }
}
