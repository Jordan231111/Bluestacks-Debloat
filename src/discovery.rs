use crate::{config::Config, platform};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use sysinfo::{ProcessesToUpdate, System};
use winreg::{RegKey, enums::*};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installation {
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
    pub version: String,
    pub source: String,
}
impl Installation {
    pub fn conf(&self) -> PathBuf {
        self.data_dir.join("bluestacks.conf")
    }
    pub fn player(&self) -> PathBuf {
        self.install_dir.join("HD-Player.exe")
    }
    pub fn adb(&self) -> PathBuf {
        self.install_dir.join("HD-Adb.exe")
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.player().is_file() && self.adb().is_file() && self.conf().is_file(),
            "Installation is incomplete; select the folder containing HD-Player.exe and the data folder containing bluestacks.conf"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    pub display_name: String,
    pub adb_port: Option<u16>,
    pub cpus: u32,
    pub ram_mb: u32,
    pub fps: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub path: Option<PathBuf>,
    pub instance: Option<String>,
    pub memory_mb: u64,
    pub start_time: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub installation: Installation,
    pub instances: Vec<Instance>,
    pub processes: Vec<Process>,
    pub cpu_count: usize,
    pub ram_mb: u64,
    pub admin: bool,
}

pub fn valid_instance(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 100
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn data_root(path: &Path) -> Option<PathBuf> {
    let mut p = if path.is_file() {
        path.parent()?.to_owned()
    } else {
        path.to_owned()
    };
    for _ in 0..5 {
        if p.join("bluestacks.conf").is_file() {
            return platform::absolute(&p).ok();
        }
        if !p.pop() {
            break;
        }
    }
    None
}
pub fn installations() -> Result<Vec<Installation>> {
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (hive, prefix) in [(HKEY_LOCAL_MACHINE, "HKLM"), (HKEY_CURRENT_USER, "HKCU")] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let Ok(software) =
                RegKey::predef(hive).open_subkey_with_flags("SOFTWARE", KEY_READ | view)
            else {
                continue;
            };
            for name in software
                .enum_keys()
                .flatten()
                .filter(|n| n.to_ascii_lowercase().starts_with("bluestacks"))
            {
                let Ok(key) = software.open_subkey(&name) else {
                    continue;
                };
                let Ok(install) = key.get_value::<String, _>("InstallDir") else {
                    continue;
                };
                let root = ["UserDefinedDir", "DataDir"]
                    .iter()
                    .filter_map(|n| key.get_value::<String, _>(*n).ok())
                    .find_map(|p| data_root(Path::new(p.trim_matches('"'))));
                let Some(data_dir) = root else { continue };
                let Ok(install_dir) = platform::absolute(Path::new(install.trim_matches('"')))
                else {
                    continue;
                };
                let item = Installation {
                    install_dir,
                    data_dir,
                    version: key
                        .get_value("Version")
                        .unwrap_or_else(|_| "unknown".into()),
                    source: format!("{prefix}\\SOFTWARE\\{name} ({view})"),
                };
                if item.validate().is_ok()
                    && seen.insert(
                        format!("{}|{}", item.install_dir.display(), item.data_dir.display())
                            .to_lowercase(),
                    )
                {
                    result.push(item);
                }
            }
        }
    }
    Ok(result)
}
pub fn select(install: Option<&Path>, conf: Option<&Path>) -> Result<Installation> {
    let mut found = installations()?;
    if let (Some(install), Some(conf)) = (install, conf) {
        let item = Installation {
            install_dir: platform::absolute(install)?,
            data_dir: data_root(conf).context("No bluestacks.conf in the selected data path")?,
            version: "custom".into(),
            source: "explicit paths".into(),
        };
        item.validate()?;
        if let Some(registered) = found.into_iter().find(|registered| {
            platform::same_path(&registered.install_dir, &item.install_dir)
                && platform::same_path(&registered.data_dir, &item.data_dir)
        }) {
            return Ok(registered);
        }
        return Ok(item);
    }
    if let Some(path) = install {
        found.retain(|i| platform::same_path(&i.install_dir, path));
    }
    if let Some(path) = conf {
        let root = data_root(path).context("No configuration at the specified path")?;
        found.retain(|i| platform::same_path(&i.data_dir, &root));
    }
    ensure!(
        found.len() == 1,
        "Found {} matching installations. Select both install and data folders to resolve missing or ambiguous registry records.",
        found.len()
    );
    Ok(found.remove(0))
}
pub fn instances(install: &Installation, conf: &Config) -> Vec<Instance> {
    let names: BTreeSet<_> = conf
        .keys()
        .filter_map(|k| {
            k.strip_prefix("bst.instance.")?
                .split_once('.')
                .map(|v| v.0)
        })
        .filter(|n| valid_instance(n))
        .collect();
    names
        .into_iter()
        .filter(|name| {
            install
                .data_dir
                .join("Engine")
                .join(name)
                .join(format!("{name}.bstk"))
                .is_file()
        })
        .map(|name| {
            let get = |suffix: &str| conf.get(&format!("bst.instance.{name}.{suffix}"));
            Instance {
                name: name.into(),
                display_name: get("display_name").unwrap_or(name).into(),
                adb_port: get("status.adb_port")
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|p| *p > 0)
                    .or_else(|| {
                        get("adb_port")
                            .and_then(|s| s.parse().ok())
                            .filter(|p| *p > 0)
                    }),
                cpus: get("cpus").and_then(|s| s.parse().ok()).unwrap_or(0),
                ram_mb: get("ram").and_then(|s| s.parse().ok()).unwrap_or(0),
                fps: get("max_fps").and_then(|s| s.parse().ok()).unwrap_or(0),
            }
        })
        .collect()
}
pub fn log_tail(install: &Installation) -> Result<String> {
    let mut f = fs::File::open(install.data_dir.join("Logs").join("Player.log"))?;
    let len = f.metadata()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(2 * 1024 * 1024)))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
pub fn processes(install: &Installation) -> Result<Vec<Process>> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    // The player protects its executable path/command line on recent builds. Correlate
    // timestamped instance logs with process birth time, so recycled PIDs cannot match.
    let re = regex::Regex::new(r"(?m)^(\S+\s+\S+)\s+(\d+)\s+\d+\s+\S+\s+([A-Za-z0-9_]+)\s+\[")?;
    let mut logged = BTreeMap::new();
    for c in re.captures_iter(&log_tail(install).unwrap_or_default()) {
        if let (Ok(pid), Ok(time)) = (
            c[2].parse::<u32>(),
            chrono::DateTime::parse_from_str(&c[1], "%Y-%m-%d %H:%M:%S%.f%z"),
        ) {
            logged.insert(pid, (time.timestamp(), c[3].to_owned()));
        }
    }
    let mut out = Vec::new();
    for (pid, p) in sys.processes() {
        if pid.as_u32() == std::process::id()
            || p.name().eq_ignore_ascii_case("BluestacksDebloat.exe")
        {
            continue;
        }
        let name = p.name().to_string_lossy().into_owned();
        let path = p.exe().map(Path::to_owned);
        let exact = path
            .as_ref()
            .and_then(|p| p.parent())
            .is_some_and(|p| platform::same_path(p, &install.install_dir));
        let instance = logged
            .get(&pid.as_u32())
            .filter(|(time, _)| *time >= p.start_time() as i64 - 2)
            .map(|(_, n)| n.clone());
        if exact || name.eq_ignore_ascii_case("HD-Player.exe") && instance.is_some() {
            out.push(Process {
                pid: pid.as_u32(),
                name,
                path,
                instance,
                memory_mb: p.memory() / 1024 / 1024,
                start_time: p.start_time(),
            });
        } else if name.eq_ignore_ascii_case("HD-Player.exe") && path.is_none() {
            // Include unknown protected players to veto offline writes, but never target
            // one with ADB until positive instance evidence is available.
            out.push(Process {
                pid: pid.as_u32(),
                name,
                path,
                instance: None,
                memory_mb: p.memory() / 1024 / 1024,
                start_time: p.start_time(),
            });
        }
    }
    Ok(out)
}
pub fn snapshot(installation: Installation) -> Result<Snapshot> {
    installation.validate()?;
    let conf = Config::read(&installation.conf())?;
    let instances = instances(&installation, &conf);
    let processes = processes(&installation)?;
    let mut sys = System::new();
    sys.refresh_memory();
    Ok(Snapshot {
        installation,
        instances,
        processes,
        cpu_count: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        ram_mb: sys.total_memory() / 1024 / 1024,
        admin: platform::is_admin(),
    })
}
pub fn require_stopped(install: &Installation) -> Result<()> {
    let running: Vec<_> = processes(install)?
        .into_iter()
        .filter(|p| !p.name.eq_ignore_ascii_case("HD-Adb.exe"))
        .collect();
    ensure!(
        running.is_empty(),
        "Close BlueStacks and its Multi-instance Manager before host changes. Still running: {}",
        running
            .iter()
            .map(|p| format!("{} (PID {})", p.name, p.pid))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}
pub fn launch(install: &Installation, name: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    ensure!(
        instances(install, &Config::read(&install.conf())?)
            .iter()
            .any(|i| i.name == name),
        "Unknown instance"
    );
    std::process::Command::new(install.player())
        .args(["--instance", name])
        .creation_flags(0x00000208) // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
