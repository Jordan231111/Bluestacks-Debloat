use crate::{
    adb::{self, Client, Package},
    config::{Config, Edit},
    discovery::{self, Installation, Snapshot},
    network, patch, rules,
    transaction::{self, Operation, Plan, RegistryPath, Target, Value},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, clap::ValueEnum)]
pub enum Performance {
    #[default]
    Keep,
    Balanced,
    Gaming,
    LowMemory,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostOptions {
    pub ads: bool,
    pub telemetry: bool,
    pub smart_downloads: bool,
    pub cloud: bool,
    pub quiet: bool,
    pub performance: Performance,
    pub gpu: bool,
    pub high_fps: bool,
    pub hosts: bool,
    pub patch: bool,
    pub enable_adb: bool,
    pub remove_x: bool,
    pub remove_services: bool,
}
impl Default for HostOptions {
    fn default() -> Self {
        Self {
            ads: true,
            telemetry: true,
            smart_downloads: true,
            cloud: false,
            quiet: false,
            performance: Performance::Keep,
            gpu: false,
            high_fps: false,
            hosts: false,
            patch: false,
            enable_adb: false,
            remove_x: false,
            remove_services: false,
        }
    }
}
impl HostOptions {
    pub fn maximum() -> Self {
        Self {
            cloud: true,
            quiet: true,
            remove_x: true,
            remove_services: true,
            gpu: true,
            high_fps: true,
            hosts: true,
            enable_adb: true,
            ..Self::default()
        }
    }
}

pub fn host_plan(snapshot: &Snapshot, instance: &str, options: &HostOptions) -> Result<Plan> {
    let install = &snapshot.installation;
    install.validate()?;
    ensure!(
        snapshot.instances.iter().any(|i| i.name == instance),
        "Select an existing Android instance"
    );
    let config = Config::read(&install.conf())?;
    let mut changes = BTreeMap::<String, Edit>::new();
    let mut notes = Vec::new();
    let mut set = |key: &str, value: &str| {
        if let Some(before) = config.get(key)
            && before != value
        {
            changes.insert(
                key.into(),
                Edit {
                    key: key.into(),
                    before: before.into(),
                    after: value.into(),
                },
            );
        }
    };
    for (enabled, keys) in [
        (options.ads, rules::ADS),
        (options.telemetry, rules::STATS),
        (options.smart_downloads, rules::DOWNLOADS),
        (options.cloud || options.remove_x, rules::CLOUD),
    ] {
        if enabled {
            for key in keys {
                match config.get(key) {
                    Some("1") => set(key, "0"),
                    Some("true") => set(key, "false"),
                    Some("0" | "false") | None => {}
                    Some(_) => notes.push(format!("Skipped {key}: unsupported non-boolean value")),
                }
            }
        }
    }
    let instance_key = |name: &str| format!("bst.instance.{instance}.{name}");
    if options.ads {
        set(&instance_key("split_ad_enabled"), "0");
    }
    if options.quiet {
        set(&instance_key("enable_notifications"), "0");
        set(&instance_key("enable_mobile_notifications"), "0");
    }
    if options.enable_adb {
        set("bst.enable_adb_access", "1");
        set("bst.enable_adb_remote_access", "0");
    }
    if options.gpu {
        set("bst.prefer_dedicated_gpu", "1");
    }
    if options.performance != Performance::Keep {
        let cpu_budget = (snapshot.cpu_count / 2).max(1) as u32;
        let mem_budget = (snapshot.ram_mb / 3).max(1024) as u32;
        let (cores, ram, fps) = match options.performance {
            Performance::Balanced => (4, 4096, 60),
            Performance::Gaming => (8, 8192, 60),
            Performance::LowMemory => (2, 2048, 30),
            Performance::Keep => unreachable!(),
        };
        set(&instance_key("cpus"), &cores.min(cpu_budget).to_string());
        set(&instance_key("ram"), &ram.min(mem_budget).to_string());
        set(&instance_key("max_fps"), &fps.to_string());
        set(&instance_key("enable_high_fps"), "0");
        notes.push("CPU/RAM limits apply only to the selected instance and reserve resources for Windows. Renderer and resolution stay as configured. FPS improvements depend on the game and hardware.".into());
    }
    if options.high_fps {
        set(&instance_key("enable_high_fps"), "1");
        set(&instance_key("max_fps"), "240");
        set(&instance_key("enable_vsync"), "0");
        notes.push("High-frame-rate mode raises the limit to 240 FPS and disables emulator VSync. Actual FPS depends on the game, display and GPU; power use can increase. CPU cores and RAM are not changed by this option.".into());
    }
    let mut operations = Vec::new();
    let edits: Vec<_> = changes.into_values().collect();
    if !edits.is_empty() {
        let after = config.edit(&edits, false)?;
        operations.push(Operation {
            label: format!("Update {} existing BlueStacks settings", edits.len()),
            target: Target::Config {
                path: install.conf(),
                edits,
            },
            before: Value::Bytes(config.bytes().into()),
            after: Value::Bytes(after),
        });
    }
    if options.gpu {
        let p = RegistryPath {
            machine: false,
            view32: false,
            key: r"Software\Microsoft\DirectX\UserGpuPreferences".into(),
            name: install.player().to_string_lossy().into(),
        };
        let before = transaction::read_registry(&p)?;
        let after = transaction::string_value("GpuPreference=2;");
        if before != after {
            operations.push(Operation {
                label: "Prefer the high-performance GPU for HD-Player (Windows preference)".into(),
                target: Target::Registry(p),
                before,
                after,
            });
        }
    }
    if options.hosts {
        let path = network::hosts_path();
        let bytes = fs::read(&path)?;
        let after = network::hosts_block(&bytes, network::DOMAINS)?;
        if bytes != after {
            operations.push(Operation{label:"Block the listed advertising domains in Windows hosts (affects all Windows apps)".into(),target:Target::File{path},before:Value::Bytes(bytes),after:Value::Bytes(after)});
        }
        notes.push("Hosts filtering is optional and system-wide. It does not guarantee filtering inside Android, or cover encrypted DNS, hard-coded IPs or every ad provider. Rewarded in-game ads can stop working.".into());
    }
    if options.patch {
        let path = install.player();
        let before = fs::read(&path)?;
        let (after, report) = patch::patched(&before)?;
        notes.push(format!("Integrity patch: {} candidate site(s), {} already patched. Changes affect all instances using this player and invalidate its Authenticode signature.",report.offsets.len(),report.already_patched));
        if before != after {
            operations.push(Operation {
                label: format!("Apply disk-integrity patch at {:?}", report.offsets),
                target: Target::File { path },
                before: Value::Bytes(before),
                after: Value::Bytes(after),
            });
        }
    }
    if options.remove_x || options.remove_services {
        crate::cloud::add_removal(
            install,
            options.remove_x,
            options.remove_services,
            &mut operations,
            &mut notes,
        )?;
    }
    notes.push("Host-wide settings affect all instances. Instance settings affect only the selected instance. Close BlueStacks before applying; the preview itself does not stop processes or change settings.".into());
    Ok(Plan {
        installation: install.clone(),
        guest: None,
        title: format!("Host debloat • {instance}"),
        operations,
        notes,
    })
}
pub fn guest_scan(install: &Installation, instance: &str) -> Result<(adb::Guest, Vec<Package>)> {
    let adb = Client::connect(install, instance)?;
    let packages = adb.candidates()?;
    Ok((adb.guest.clone(), packages))
}
pub fn guest_plan(
    install: &Installation,
    instance: &str,
    selected: &[String],
    animations: bool,
) -> Result<Plan> {
    let adb = Client::connect(install, instance)?;
    let candidates = adb.candidates()?;
    let mut operations = Vec::new();
    for name in selected {
        let package = candidates
            .iter()
            .find(|p| &p.name == name)
            .context("Requested package is outside the reviewed candidate list")?;
        if package.enabled <= 1 {
            operations.push(Operation {
                label: format!("Disable {name} — {}", package.reason),
                target: Target::Package { name: name.clone() },
                before: Value::Number(package.enabled),
                after: Value::Number(3),
            });
        }
    }
    if animations {
        for name in adb::ANIMATIONS {
            let before = adb.setting(name)?;
            let after = Some("0.5".into());
            if before != after {
                operations.push(Operation {
                    label: format!("Shorten Android UI animation: {name}"),
                    target: Target::Setting {
                        name: (*name).into(),
                    },
                    before: Value::Text(before),
                    after: Value::Text(after),
                });
            }
        }
    }
    Ok(Plan{installation:install.clone(),guest:Some(adb.guest.clone()),title:format!("Android debloat • {instance}"),operations,notes:vec!["Keep this instance running while applying or restoring Android changes. Disabling preserves app data; restore returns each package to its exact previous enabled state.".into(),"The launcher, Google Play, accounts, billing, keyboard and installed games are excluded from automatic package disabling. Shorter UI animations do not increase game FPS.".into()]})
}
pub fn refresh(install: &Installation) -> Result<Snapshot> {
    discovery::snapshot(install.clone())
}

pub fn root_plan(
    install: &Installation,
    instance: &str,
    hosts: bool,
    isolate: bool,
) -> Result<Plan> {
    ensure!(
        hosts || isolate,
        "Select Android hosts filtering and/or launcher isolation"
    );
    let adb = Client::connect(install, instance)?;
    let operations = crate::rooted::operation(&adb, hosts, isolate)?
        .into_iter()
        .collect();
    Ok(Plan{installation:install.clone(),guest:Some(adb.guest.clone()),title:format!("Root network controls • {instance}"),operations,notes:vec!["Requires working Magisk. The module is stored in this instance's /data; shared Root.vhd is untouched. Restart the instance to activate or remove the hosts overlay.".into(),"Launcher isolation blocks only com.uncube.launcher3's Internet traffic, for both IPv4 and IPv6. Launcher search, promotions and online store functions stop working; Google Play and game packages keep their own connections. Local host-bridge traffic remains allowed.".into(),"Restart Android after applying to refresh the launcher. Restore removes the owned firewall chain immediately; restart Android after restoring a hosts overlay. Existing unrelated firewall rules are preserved.".into()]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_rules_do_not_guess_unknown_keys() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("HD-Player.exe"), []).unwrap();
        fs::write(t.path().join("HD-Adb.exe"), []).unwrap();
        fs::write(t.path().join("bluestacks.conf"),b"bst.enable_programmatic_ads=\"1\"\nbst.unknown_ad_loader=\"1\"\nbst.enable_adb_access=\"1\"\nbst.instance.Pie64.split_ad_enabled=\"1\"\nbst.instance.Rvc64.split_ad_enabled=\"1\"\n").unwrap();
        let install = Installation {
            install_dir: t.path().into(),
            data_dir: t.path().into(),
            version: "test".into(),
            source: "test".into(),
        };
        let snapshot = Snapshot {
            installation: install,
            instances: vec![discovery::Instance {
                name: "Pie64".into(),
                display_name: "test".into(),
                adb_port: Some(5555),
                cpus: 2,
                ram_mb: 2048,
                fps: 60,
            }],
            processes: vec![],
            cpu_count: 16,
            ram_mb: 32768,
            admin: false,
        };
        let plan = host_plan(&snapshot, "Pie64", &HostOptions::default()).unwrap();
        let Value::Bytes(after) = &plan.operations[0].after else {
            panic!()
        };
        let c = Config::parse(String::from_utf8(after.clone()).unwrap()).unwrap();
        assert_eq!(c.get("bst.enable_programmatic_ads"), Some("0"));
        assert_eq!(c.get("bst.unknown_ad_loader"), Some("1"));
        assert_eq!(c.get("bst.enable_adb_access"), Some("1"));
        assert_eq!(c.get("bst.instance.Rvc64.split_ad_enabled"), Some("1"));
    }
    #[test]
    fn maximum_keeps_cpu_ram_and_other_instance_allocations() {
        let t = tempfile::tempdir().unwrap();
        for name in ["HD-Player.exe", "HD-Adb.exe"] {
            fs::write(t.path().join(name), []).unwrap();
        }
        fs::write(t.path().join("bluestacks.conf"),"bst.enable_programmatic_ads=\"1\"\nbst.instance.Pie64.cpus=\"3\"\nbst.instance.Pie64.ram=\"3072\"\nbst.instance.Pie64.enable_high_fps=\"0\"\nbst.instance.Pie64.max_fps=\"60\"\nbst.instance.Pie64.enable_vsync=\"1\"\nbst.instance.Rvc64.cpus=\"2\"\nbst.instance.Rvc64.ram=\"2048\"\n").unwrap();
        let snapshot = Snapshot {
            installation: Installation {
                install_dir: t.path().into(),
                data_dir: t.path().into(),
                version: "test".into(),
                source: "test".into(),
            },
            instances: vec![discovery::Instance {
                name: "Pie64".into(),
                display_name: "test".into(),
                adb_port: Some(5555),
                cpus: 3,
                ram_mb: 3072,
                fps: 60,
            }],
            processes: vec![],
            cpu_count: 16,
            ram_mb: 32768,
            admin: false,
        };
        let mut options = HostOptions::maximum();
        options.gpu = false;
        options.hosts = false;
        options.remove_x = false;
        options.remove_services = false;
        let plan = host_plan(&snapshot, "Pie64", &options).unwrap();
        let Value::Bytes(after) = &plan.operations[0].after else {
            panic!()
        };
        let c = Config::parse(String::from_utf8(after.clone()).unwrap()).unwrap();
        assert_eq!(c.get("bst.instance.Pie64.cpus"), Some("3"));
        assert_eq!(c.get("bst.instance.Pie64.ram"), Some("3072"));
        assert_eq!(c.get("bst.instance.Rvc64.cpus"), Some("2"));
        assert_eq!(c.get("bst.instance.Rvc64.ram"), Some("2048"));
        assert_eq!(c.get("bst.instance.Pie64.max_fps"), Some("240"));
        assert_eq!(c.get("bst.instance.Pie64.enable_high_fps"), Some("1"));
        assert_eq!(c.get("bst.instance.Pie64.enable_vsync"), Some("0"));
    }
}
