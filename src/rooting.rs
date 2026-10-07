//! Native port of the companion Magisk Prep → Data → Clean → Finalize → Verify
//! workflow. Guest templates and checked tools are embedded; no .cmd/PS engine runs.
use crate::{
    adb::{Client, quote},
    config::{Config, Edit},
    discovery::{self, Installation},
    patch, platform,
    root_assets::{self, Assets},
    root_files::{self, Backup},
    virtual_disk::Disk,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootInfo {
    pub installation: Installation,
    pub instance: String,
    pub system_disk: PathBuf,
    pub data_disk: PathBuf,
    pub machine_file: PathBuf,
    pub master_file: Option<PathBuf>,
    pub shared_instances: Vec<String>,
    pub backup_bytes: u64,
    pub supported: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub instance: String,
    pub root: bool,
    pub magisk_version: String,
    pub selinux: String,
    pub su_target: String,
    pub competing_su: Vec<String>,
    pub bootstrap_traces: Vec<String>,
    pub complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootBackupInfo {
    pub path: PathBuf,
    pub instance: String,
    pub created: String,
    pub action: String,
    pub stage: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    schema: u32,
    id: String,
    created: String,
    action: String,
    stage: String,
    info: RootInfo,
    backups: Vec<Backup>,
    identity: Option<String>,
    error: Option<String>,
}
struct CompanionLock(HANDLE);
impl CompanionLock {
    fn take() -> Result<Self> {
        let h = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                platform::wide(r"Local\BlueStackRoot.Magisk").as_ptr(),
            )
        };
        ensure!(
            !h.is_null(),
            "Could not open the companion root-operation lock"
        );
        let result = unsafe { WaitForSingleObject(h, 0) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe {
                CloseHandle(h);
            }
            bail!("Another root operation or the companion tool is running");
        }
        Ok(Self(h))
    }
}
impl Drop for CompanionLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

fn disk_from_xml(bstk: &Path, filename: &str) -> Result<PathBuf> {
    let text = fs::read_to_string(bstk)?;
    let re = regex::Regex::new(r#"\blocation="([^"]+)""#)?;
    let mut found: Vec<PathBuf> = Vec::new();
    for c in re.captures_iter(&text) {
        let value = c[1]
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace('/', "\\");
        let p = PathBuf::from(value);
        if p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(filename))
        {
            let path = if p.is_absolute() {
                p
            } else {
                bstk.parent().context("Machine file parent")?.join(p)
            };
            if path.is_file() {
                let path = platform::absolute(&path)?;
                if !found.iter().any(|p| platform::same_path(p, &path)) {
                    found.push(path);
                }
            }
        }
    }
    ensure!(
        found.len() == 1,
        "Expected one existing {filename} in {}",
        bstk.display()
    );
    Ok(found.remove(0))
}
pub fn info(install: &Installation, instance: &str) -> Result<RootInfo> {
    install.validate()?;
    ensure!(discovery::valid_instance(instance), "Invalid instance name");
    let config = Config::read(&install.conf())?;
    ensure!(
        discovery::instances(install, &config)
            .iter()
            .any(|i| i.name == instance),
        "Instance is missing or has not been initialized"
    );
    let base = instance.split('_').next().unwrap_or(instance);
    let supported = ["Pie64", "Rvc64", "Tiramisu64"].contains(&base);
    let machine_file = install
        .data_dir
        .join("Engine")
        .join(instance)
        .join(format!("{instance}.bstk"));
    let system_disk = disk_from_xml(&machine_file, "Root.vhd")?;
    let data_disk = disk_from_xml(&machine_file, "Data.vhdx")?;
    let parent = system_disk.parent().context("System disk has no parent")?;
    let master_file = parent.join(format!(
        "{}.bstk",
        parent.file_name().unwrap_or_default().to_string_lossy()
    ));
    let master_file = master_file.is_file().then_some(master_file);
    let mut shared_instances = Vec::new();
    for i in discovery::instances(install, &config) {
        let b = install
            .data_dir
            .join("Engine")
            .join(&i.name)
            .join(format!("{}.bstk", i.name));
        if disk_from_xml(&b, "Root.vhd").is_ok_and(|p| platform::same_path(&p, &system_disk)) {
            shared_instances.push(i.name);
        }
    }
    let backup_bytes = system_disk.metadata()?.len()
        + data_disk.metadata()?.len()
        + install.player().metadata()?.len();
    Ok(RootInfo {
        installation: install.clone(),
        instance: instance.into(),
        system_disk,
        data_disk,
        machine_file,
        master_file,
        shared_instances,
        backup_bytes,
        supported,
    })
}
fn save(dir: &Path, j: &Journal) -> Result<()> {
    platform::write_journal(&dir.join("root-journal.json"), j)
}
fn stage(dir: &Path, j: &mut Journal, name: &str, log: &mut impl FnMut(String)) -> Result<()> {
    j.stage = name.into();
    save(dir, j)?;
    log(name.into());
    Ok(())
}
fn set_conf(info: &RootInfo, root: bool) -> Result<()> {
    let path = info.installation.conf();
    let c = Config::read(&path)?;
    let flag = if root { "1" } else { "0" };
    let values = [
        (
            format!("bst.instance.{}.enable_root_access", info.instance),
            flag,
        ),
        ("bst.feature.rooting".into(), flag),
        ("bst.enable_adb_access".into(), "1"),
        ("bst.enable_adb_remote_access".into(), "0"),
    ];
    let mut edits = Vec::new();
    for (key, after) in values {
        if let Some(before) = c.get(&key) {
            if before != after {
                edits.push(Edit {
                    key,
                    before: before.into(),
                    after: after.into(),
                });
            }
        } else {
            ensure!(
                key == "bst.enable_adb_remote_access",
                "Required root configuration is missing: {key}"
            );
        }
    }
    platform::atomic_write(&path, &c.edit(&edits, false)?)
}

pub fn stop_players(install: &Installation, log: &mut impl FnMut(String)) -> Result<()> {
    let initial = discovery::processes(install)?;
    for p in &initial {
        if p.name.eq_ignore_ascii_case("HD-Player.exe")
            && let Some(name) = &p.instance
            && let Ok(adb) = Client::connect(install, name)
        {
            let _ = adb.idempotent_shell("sync");
        }
    }
    let ids = initial
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("HD-Player.exe"))
        .map(|p| p.pid)
        .collect::<Vec<_>>();
    platform::close_windows(&ids);
    if !ids.is_empty() {
        log(
            "Closing BlueStacks for offline maintenance; your last guest writes have been synced"
                .into(),
        );
        std::thread::sleep(Duration::from_millis(800));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let remaining = discovery::processes(install)?
            .into_iter()
            .filter(|p| !p.name.eq_ignore_ascii_case("HD-Adb.exe"))
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "BlueStacks did not stop; no offline edits can continue"
        );
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        for p in remaining {
            ensure!(
                [
                    "HD-Player.exe",
                    "HD-MultiInstanceManager.exe",
                    "BstkSVC.exe",
                    "BstkVMMgr.exe",
                    "BlueStacksHelper.exe",
                    "BlueStacksAppplayerWeb.exe",
                    "HD-CommonLoader.exe"
                ]
                .iter()
                .any(|n| p.name.eq_ignore_ascii_case(n)),
                "Close {} before rooting; it may be an installer or another maintenance tool",
                p.name
            );
            ensure!(
                p.path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .is_some_and(|p| platform::same_path(p, &install.install_dir))
                    || p.instance.is_some(),
                "Cannot positively identify a protected player; close it manually"
            );
            if let Some(current) = sys.process(sysinfo::Pid::from_u32(p.pid))
                && current.start_time() == p.start_time
            {
                ensure!(current.kill(), "Could not close {}", p.name);
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
pub fn boot(info: &RootInfo, log: &mut impl FnMut(String)) -> Result<Client> {
    if !discovery::processes(&info.installation)?
        .iter()
        .any(|p| p.instance.as_deref() == Some(&info.instance))
    {
        log(format!(
            "Starting {} and waiting for Android",
            info.instance
        ));
        discovery::launch(&info.installation, &info.instance)?;
    }
    let start = Instant::now();
    let mut last = String::new();
    let mut reported = 0;
    while start.elapsed() < Duration::from_secs(360) {
        match Client::connect(&info.installation, &info.instance) {
            Ok(adb) => {
                let mut stable = true;
                for _ in 0..3 {
                    if !adb
                        .idempotent_shell("printf BSD_READY")
                        .is_ok_and(|s| s == "BSD_READY")
                    {
                        stable = false;
                        break;
                    }
                }
                if stable {
                    return Ok(adb);
                }
            }
            Err(e) => last = format!("{e:#}"),
        }
        let elapsed = start.elapsed().as_secs();
        if elapsed >= reported + 15 {
            reported = elapsed;
            log(format!(
                "Waiting for {} to finish booting ({elapsed}s): {last}",
                info.instance
            ));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    bail!("{} did not become ready: {last}", info.instance)
}
fn bootstrap(adb: &Client) -> Result<String> {
    let mut results = Vec::new();
    for path in [
        "/system/etc/bsr_su",
        "/system/xbin/su",
        "/system/xbin/bstk/su",
    ] {
        let output = adb.idempotent_shell(&format!("{} -c id 2>&1 || true", quote(path)))?;
        if output.contains("uid=0") {
            return Ok(path.into());
        }
        results.push(format!("{path}: {output}"));
    }
    bail!("Bootstrap root was not available. {}", results.join("; "))
}
fn su(adb: &Client, path: &str, script: &str) -> Result<String> {
    adb.idempotent_shell(&format!("{} -c {}", quote(path), quote(script)))
}
fn script(
    adb: &Client,
    assets: &Assets,
    name: &str,
    text: &str,
    root_path: &str,
) -> Result<String> {
    let local = assets.directory.join(name);
    platform::atomic_write(&local, text.as_bytes())?;
    let target = format!("/data/local/tmp/{name}");
    adb.push_file(&local, &target)?;
    let result = su(adb, root_path, &format!("sh {}", quote(&target)));
    if result.is_ok() {
        let _ = adb.idempotent_shell(&format!("rm -f {}", quote(&target)));
    }
    result
}

pub fn verify(info: &RootInfo, log: &mut impl FnMut(String)) -> Result<Verification> {
    let adb = boot(info, log)?;
    let su_target = adb.idempotent_shell("readlink /system/bin/su 2>/dev/null || true")?;
    let daemon = adb.idempotent_shell("pidof magiskd 2>/dev/null || true")?;
    // Factory BlueStacks su can wait indefinitely for a host daemon. Query
    // Magisk's own indicators before asking for root, so stock checks stay fast.
    let present = !daemon.trim().is_empty()
        || ["magisk", "magisk32", "magisk64"].contains(&su_target.rsplit('/').next().unwrap_or(""));
    let root = present && adb.idempotent_shell("su -c 'id -u' 2>&1 || true")?.trim() == "0";
    let selinux = adb.idempotent_shell("getenforce")?;
    if !root {
        return Ok(Verification {
            instance: info.instance.clone(),
            root: false,
            magisk_version: String::new(),
            selinux,
            su_target: String::new(),
            competing_su: Vec::new(),
            bootstrap_traces: Vec::new(),
            complete: true,
        });
    }
    let magisk_version = adb.idempotent_shell("su -c 'magisk -c'")?;
    let inventory=adb.idempotent_shell("su -c 'for f in /system/bin/su /system/xbin/su /sbin/su /vendor/bin/su /odm/bin/su /system_ext/bin/su /product/bin/su /debug_ramdisk/su; do if [ -L \"$f\" ]; then echo \"$f|link|$(readlink \"$f\")\"; elif [ -e \"$f\" ]; then echo \"$f|file|\"; fi; done'")?;
    let competing_su = inventory
        .lines()
        .filter_map(|line| {
            let p: Vec<_> = line.split('|').collect();
            if p.len() < 2 {
                return None;
            }
            let valid = p.get(2).is_some_and(|p| {
                ["magisk", "magisk32", "magisk64"].contains(&p.rsplit('/').next().unwrap_or(""))
            });
            if p[1] == "file" || !valid {
                Some(p[0].into())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let sweep = format!(
        "BB=/data/adb/magisk/busybox; test -x \"$BB\" || exit 1; set -- /system /data/adb; [ ! -d /data/downloads ] || set -- \"$@\" /data/downloads; files=$(\"$BB\" find \"$@\" -type f -size 4968c) || exit 1; printf '%s\\n' \"$files\" | while IFS= read -r f; do [ -n \"$f\" ] || continue; h=$(\"$BB\" sha256sum \"$f\") || exit 1; case \"$h\" in {}*) echo \"TRACE:$f\";; esac; done || exit 1; echo SWEEPDONE",
        root_assets::BOOTSTRAP_HASH
    );
    let sweep = su(&adb, "su", &sweep)?;
    ensure!(
        sweep.lines().any(|s| s.trim() == "SWEEPDONE"),
        "Bootstrap verification sweep did not finish"
    );
    let bootstrap_traces = sweep
        .lines()
        .filter_map(|s| s.strip_prefix("TRACE:").map(str::to_owned))
        .collect::<Vec<_>>();
    let complete = competing_su.is_empty()
        && bootstrap_traces.is_empty()
        && ["magisk", "magisk32", "magisk64"].contains(&su_target.rsplit('/').next().unwrap_or(""));
    Ok(Verification {
        instance: info.instance.clone(),
        root,
        magisk_version,
        selinux,
        su_target,
        competing_su,
        bootstrap_traces,
        complete,
    })
}
fn readonly_machine(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)?;
    let tag = regex::Regex::new(r"(?s)<HardDisk\b[^>]*>")?;
    let loc = regex::Regex::new(r#"\blocation="([^"]+)""#)?;
    let mode = regex::Regex::new(r#"\btype="Normal""#)?;
    let updated = tag.replace_all(&raw, |c: &regex::Captures| {
        let text = &c[0];
        if loc.captures(text).is_some_and(|v| {
            ["root.vhd", "fastboot.vdi"].contains(
                &v[1]
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .as_str(),
            )
        }) {
            mode.replace_all(text, "type=\"Readonly\"").into_owned()
        } else {
            text.into()
        }
    });
    if updated != raw {
        platform::atomic_write(path, updated.as_bytes())?;
    }
    Ok(())
}

fn prepare_journal(
    info: &RootInfo,
    action: &str,
    state: &Path,
    log: &mut impl FnMut(String),
) -> Result<(PathBuf, Journal)> {
    let id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let dir = state.join("root-backups").join(&id);
    fs::create_dir_all(&dir)?;
    let mut j = Journal {
        schema: 1,
        id,
        created: chrono::Utc::now().to_rfc3339(),
        action: action.into(),
        stage: "Preparing recovery copies".into(),
        info: info.clone(),
        backups: Vec::new(),
        identity: None,
        error: None,
    };
    save(&dir, &j)?;
    let mut paths = vec![
        info.system_disk.clone(),
        info.data_disk.clone(),
        info.installation.player(),
        info.installation.conf(),
        info.machine_file.clone(),
    ];
    if let Some(p) = &info.master_file {
        paths.push(p.clone());
    }
    let mut seen = BTreeSet::new();
    paths.retain(|p| seen.insert(p.to_string_lossy().to_ascii_lowercase()));
    for (i, path) in paths.iter().enumerate() {
        log(format!(
            "Creating a recovery copy of {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ));
        j.backups.push(root_files::backup(
            path,
            &dir.join(format!("{i:02}.backup")),
            log,
        )?);
        save(&dir, &j)?;
    }
    j.stage = "Recovery copies verified".into();
    save(&dir, &j)?;
    Ok((dir, j))
}
fn restore_config(backup: &Backup, instance: &str, all_instances: bool) -> Result<()> {
    let bytes = fs::read(&backup.saved)?;
    ensure!(
        bytes.len() as u64 == backup.length && platform::hash(&bytes) == backup.sha256,
        "Configuration recovery copy is incomplete or changed: {}",
        backup.saved.display()
    );
    let original = Config::parse(String::from_utf8(bytes)?)?;
    let current = Config::read(&backup.source)?;
    let mut keys = vec![
        format!("bst.instance.{instance}.enable_root_access"),
        "bst.feature.rooting".into(),
        "bst.enable_adb_access".into(),
        "bst.enable_adb_remote_access".into(),
    ];
    if all_instances {
        keys.extend(
            original
                .keys()
                .filter(|k| k.starts_with("bst.instance.") && k.ends_with(".enable_root_access"))
                .cloned(),
        );
    }
    keys.sort();
    keys.dedup();
    let mut edits = Vec::new();
    for key in keys {
        if let Some(after) = original.get(&key) {
            let before = current
                .get(&key)
                .with_context(|| format!("Configuration recovery target is missing: {key}"))?;
            edits.push(Edit {
                key,
                before: before.into(),
                after: after.into(),
            });
        }
    }
    platform::atomic_write(&backup.source, &current.edit(&edits, false)?)
}
fn rollback(dir: &Path, j: &mut Journal, log: &mut impl FnMut(String)) -> Result<()> {
    stop_players(&j.info.installation, log)?;
    let mut failures = Vec::new();
    for b in j.backups.iter().rev() {
        let result = (|| {
            if b.source.is_file()
                && b.source.metadata()?.len() == b.length
                && root_files::hash_file(&b.source)? == b.sha256
            {
                return Ok(());
            }
            if platform::same_path(&b.source, &j.info.installation.conf()) {
                restore_config(
                    b,
                    &j.info.instance,
                    j.action == "Restore all shared root files",
                )
            } else {
                root_files::restore(b, log)
            }
        })();
        if let Err(e) = result {
            failures.push(format!("{}: {e:#}", b.source.display()));
        }
    }
    j.stage = if failures.is_empty() {
        "Restored"
    } else {
        "Recovery needs attention"
    }
    .into();
    j.error = (!failures.is_empty()).then(|| failures.join("\n"));
    save(dir, j)?;
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

pub fn install(
    info: RootInfo,
    repair: bool,
    state: &Path,
    mut log: impl FnMut(String),
) -> Result<Verification> {
    platform::require_admin()?;
    ensure!(
        info.supported,
        "Rooting supports Android 9, 11 and 13 64-bit instances"
    );
    let _lock = platform::lock(state)?;
    let _companion = CompanionLock::take()?;
    if !repair && Config::read(&info.installation.conf())?.get("bst.enable_adb_access") == Some("1")
    {
        log("Checking whether this instance already has working Magisk root".into());
        if let Ok(report) = verify(&info, &mut log)
            && report.root
            && report.complete
        {
            log("Magisk is already installed and verified".into());
            return Ok(report);
        }
    }
    root_assets::verify_payloads()?;
    stop_players(&info.installation, &mut log)?;
    let mut disk = Disk::attach(&info.system_disk, true)?;
    let region = disk.ext4()?;
    disk.detach()?;
    let required = info.backup_bytes + region.length + 512 * 1024 * 1024;
    let mut free = 0u64;
    ensure!(
        unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                platform::wide(state).as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0,
        "Cannot check available backup space"
    );
    ensure!(
        free >= required,
        "Rooting needs about {:.1} GiB free for backups and staging; {:.1} GiB is available",
        required as f64 / 1073741824.0,
        free as f64 / 1073741824.0
    );
    let (dir, mut journal) = prepare_journal(&info, "Root / repair", state, &mut log)?;
    let work = state
        .join("root-work")
        .join(uuid::Uuid::new_v4().to_string());
    let assets = root_assets::extract(&work)?;
    let result = (|| {
        stage(
            &dir,
            &mut journal,
            "1 / 6 — Prepare the player and system disk",
            &mut log,
        )?;
        let player = fs::read(info.installation.player())?;
        let (patched, _) = patch::patched(&player)?;
        if patched != player {
            platform::atomic_write(&info.installation.player(), &patched)?;
        }
        let factory = PathBuf::from(format!("{}.bsrbak", info.system_disk.display()));
        if !factory.exists() {
            root_files::copy_file(&info.system_disk, &factory, &mut log)?;
        }
        let player_backup = PathBuf::from(format!("{}.bak", info.installation.player().display()));
        if !player_backup.exists() {
            platform::atomic_write(&player_backup, &player)?;
        }
        set_conf(&info, true)?;
        root_files::edit_system(
            &info.system_disk,
            &assets,
            &root_files::prep_files(&assets),
            &["/android/system/xbin/daemonsu"],
            &mut log,
        )?;
        stage(
            &dir,
            &mut journal,
            "2 / 6 — Install Magisk in this Android instance",
            &mut log,
        )?;
        {
            let adb = boot(&info, &mut log)?;
            journal.identity = Some(adb.guest.identity.clone());
            save(&dir, &journal)?;
            let root = bootstrap(&adb)?;
            adb.install_apk(&assets.apk)?;
            adb.idempotent_shell("mkdir -p /data/local/tmp/bsrmbin")?;
            for (_, name) in root_assets::MAGISK_FILES {
                adb.push_file(
                    &assets.databin.join(name),
                    &format!("/data/local/tmp/bsrmbin/{name}"),
                )?;
            }
            let output = script(&adb, &assets, "bsr_pop.sh", POPULATE, &root)?;
            ensure!(
                output.contains("BSR_DATA_OK"),
                "Magisk data population did not complete"
            );
            adb.idempotent_shell("rm -rf /data/local/tmp/bsrmbin; sync")?;
        }
        stop_players(&info.installation, &mut log)?;
        stage(
            &dir,
            &mut journal,
            "3 / 6 — Initialize Magisk and grant local shell access",
            &mut log,
        )?;
        {
            let adb = boot(&info, &mut log)?;
            ensure!(
                Some(&adb.guest.identity) == journal.identity.as_ref(),
                "Instance identity changed during rooting"
            );
            let root = bootstrap(&adb)?;
            let output = script(&adb, &assets, "bsr_policy.sh", POLICY, &root)?;
            ensure!(
                output.contains("BSR_POLICY_OK"),
                "Magisk root policy could not be verified"
            );
            su(&adb, &root, "sync")?;
        }
        stop_players(&info.installation, &mut log)?;
        stage(
            &dir,
            &mut journal,
            "4 / 6 — Remove temporary root helpers",
            &mut log,
        )?;
        root_files::edit_system(
            &info.system_disk,
            &assets,
            &root_files::clean_files(&assets),
            &[
                "/android/system/etc/bsr_su",
                "/android/system/xbin/su",
                "/android/system/xbin/daemonsu",
            ],
            &mut log,
        )?;
        stage(
            &dir,
            &mut journal,
            "5 / 6 — Disable emulator root and restore shared-disk mode",
            &mut log,
        )?;
        set_conf(&info, false)?;
        readonly_machine(&info.machine_file)?;
        if let Some(p) = &info.master_file {
            readonly_machine(p)?;
        }
        stage(
            &dir,
            &mut journal,
            "6 / 6 — Cold-boot and verify Magisk-only root",
            &mut log,
        )?;
        let report = verify(&info, &mut log)?;
        ensure!(
            report.root && report.complete,
            "Magisk verification failed: {}",
            serde_json::to_string(&report)?
        );
        journal.stage = "Complete".into();
        save(&dir, &journal)?;
        log(format!("Root verified. Recovery backup: {}", dir.display()));
        Ok(report)
    })();
    if let Err(ref e) = result {
        journal.error = Some(format!("{e:#}"));
        journal.stage = "Failed — restoring recovery copies".into();
        let _ = save(&dir, &journal);
        log(format!(
            "Rooting stopped: {e:#}. Restoring the original files."
        ));
        if let Err(recovery) = rollback(&dir, &mut journal, &mut log) {
            log(format!(
                "Recovery needs attention: {recovery:#}. Keep {}",
                dir.display()
            ));
        }
    }
    // Only this freshly generated workspace is removed here; retention runs afterwards.
    if work.parent() == Some(state.join("root-work").as_path())
        && work
            .file_name()
            .is_some_and(|n| uuid::Uuid::parse_str(&n.to_string_lossy()).is_ok())
    {
        let _ = fs::remove_dir_all(&work);
    }
    crate::backup_cleanup::finish_operation(state, &_lock, &dir, &mut log);
    result
}
const POPULATE: &str = r#"set -e
mkdir -p /data/adb/magisk /data/adb/modules /data/adb/post-fs-data.d /data/adb/service.d
touch /data/adb/.bsr_root
chmod 0600 /data/adb/.bsr_root
cp -f /data/local/tmp/bsrmbin/* /data/adb/magisk/
chown -R 0:0 /data/adb/magisk
chmod 0755 /data/adb/magisk/busybox /data/adb/magisk/magisk32 /data/adb/magisk/magisk64 /data/adb/magisk/magiskboot /data/adb/magisk/magiskinit /data/adb/magisk/magiskpolicy /data/adb/magisk/*.sh
chmod 0644 /data/adb/magisk/stub.apk
restorecon -R /data/adb 2>/dev/null || true
sync
echo BSR_DATA_OK
"#;
const POLICY: &str = r#"set -e
n=0
until pidof magiskd >/dev/null; do n=$((n+1)); [ "$n" -lt 20 ] || exit 1; sleep 1; done
magisk --sqlite "REPLACE INTO policies (uid,policy,until,logging,notification) VALUES(2000,2,0,0,0)"
magisk -c
sync
echo BSR_POLICY_OK
"#;

pub fn unroot(info: RootInfo, state: &Path, mut log: impl FnMut(String)) -> Result<()> {
    platform::require_admin()?;
    ensure!(info.supported, "Unsupported Android version");
    let _lock = platform::lock(state)?;
    let _companion = CompanionLock::take()?;
    stop_players(&info.installation, &mut log)?;
    let (dir, mut journal) = prepare_journal(&info, "Unroot this instance", state, &mut log)?;
    let result = (|| {
        stage(
            &dir,
            &mut journal,
            "Remove Magisk from the selected instance",
            &mut log,
        )?;
        {
            let adb = boot(&info, &mut log)?;
            ensure!(
                adb.idempotent_shell("su -c 'id -u'")?.trim() == "0",
                "Working Magisk root is needed to verify removal. Repair root first if it is broken."
            );
            adb.idempotent_shell("su -c 'set -e; rm -f /data/adb/.bsr_root; rm -rf /data/adb/magisk /data/adb/magisk.db /data/adb/modules /data/adb/post-fs-data.d /data/adb/service.d; sync; echo BSR_UNROOT_OK'")?;
            adb.uninstall_magisk()?;
            adb.idempotent_shell("sync")?;
        }
        stop_players(&info.installation, &mut log)?;
        set_conf(&info, false)?;
        stage(
            &dir,
            &mut journal,
            "Cold-boot and confirm root is removed",
            &mut log,
        )?;
        let report = verify(&info, &mut log)?;
        ensure!(!report.root, "Root is still active after unrooting");
        journal.stage = "Complete".into();
        save(&dir, &journal)?;
        log(format!(
            "{} is unrooted. Other instances keep their root. Recovery: {}",
            info.instance,
            dir.display()
        ));
        Ok(())
    })();
    if let Err(ref error) = result {
        journal.error = Some(format!("{error:#}"));
        let _ = save(&dir, &journal);
        if let Err(e) = rollback(&dir, &mut journal, &mut log) {
            log(format!("Recovery needs attention: {e:#}"));
        }
    }
    crate::backup_cleanup::finish_operation(state, &_lock, &dir, &mut log);
    result
}

pub fn full_unroot(install: Installation, state: &Path, mut log: impl FnMut(String)) -> Result<()> {
    platform::require_admin()?;
    let _lock = platform::lock(state)?;
    let _companion = CompanionLock::take()?;
    let instances = discovery::instances(&install, &Config::read(&install.conf())?);
    ensure!(!instances.is_empty(), "No instances found");
    let infos = instances
        .iter()
        .map(|i| info(&install, &i.name))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        infos.iter().all(|i| i.supported),
        "Global system restore needs supported Android 9/11/13 instances only"
    );
    let player_backup = PathBuf::from(format!("{}.bak", install.player().display()));
    ensure!(
        player_backup.is_file(),
        "The original HD-Player.exe.bak is required for a shared-system restore"
    );
    let mut masters = Vec::<(PathBuf, PathBuf)>::new();
    for i in &infos {
        if !masters
            .iter()
            .any(|(p, _)| platform::same_path(p, &i.system_disk))
        {
            let b = PathBuf::from(format!("{}.bsrbak", i.system_disk.display()));
            ensure!(
                b.is_file(),
                "A complete original system backup is required for {}",
                i.system_disk.display()
            );
            crate::virtual_disk::format(&b)?;
            masters.push((i.system_disk.clone(), b));
        }
    }
    // Restoring only one Android master then unpatching the shared player can
    // strand the other rooted Android versions. Restore every master together.
    stop_players(&install, &mut log)?;
    let (dir, mut journal) =
        prepare_journal(&infos[0], "Restore all shared root files", state, &mut log)?;
    for i in infos.iter().skip(1) {
        for p in [&i.system_disk, &i.data_disk] {
            if !journal
                .backups
                .iter()
                .any(|b| platform::same_path(&b.source, p))
            {
                let n = journal.backups.len();
                journal.backups.push(root_files::backup(
                    p,
                    &dir.join(format!("{n:02}.backup")),
                    &mut log,
                )?);
                save(&dir, &journal)?;
            }
        }
    }
    let result = (|| {
        for i in &infos {
            stage(
                &dir,
                &mut journal,
                &format!("Remove per-instance root: {}", i.instance),
                &mut log,
            )?;
            {
                let adb = boot(i, &mut log)?;
                let has = adb.idempotent_shell("su -c 'id -u' 2>&1 || true")?.trim() == "0";
                if has {
                    adb.idempotent_shell("su -c 'set -e; rm -f /data/adb/.bsr_root; rm -rf /data/adb/magisk /data/adb/magisk.db /data/adb/modules /data/adb/post-fs-data.d /data/adb/service.d; sync'")?;
                }
                adb.uninstall_magisk()?;
                adb.idempotent_shell("sync")?;
            }
            stop_players(&install, &mut log)?;
            set_conf(i, false)?;
        }
        stage(
            &dir,
            &mut journal,
            "Restore every shared system disk and the original player",
            &mut log,
        )?;
        for (p, b) in &masters {
            root_files::copy_file(b, p, &mut log)?;
        }
        root_files::copy_file(&player_backup, &install.player(), &mut log)?;
        for i in &infos {
            stage(
                &dir,
                &mut journal,
                &format!("Verify restored Android: {}", i.instance),
                &mut log,
            )?;
            {
                let adb = boot(i, &mut log)?;
                ensure!(
                    adb.idempotent_shell("pidof magiskd 2>/dev/null || true")?
                        .trim()
                        .is_empty(),
                    "Magisk is still running in {}",
                    i.instance
                );
                ensure!(
                    !adb.idempotent_shell("pm path io.github.huskydg.magisk || true")?
                        .contains("package:"),
                    "Magisk manager is still installed in {}",
                    i.instance
                );
            }
            stop_players(&install, &mut log)?;
        }
        journal.stage = "Complete".into();
        save(&dir, &journal)?;
        log(format!(
            "Shared root files restored for {} instances. Recovery: {}",
            infos.len(),
            dir.display()
        ));
        Ok(())
    })();
    if let Err(ref e) = result {
        journal.error = Some(format!("{e:#}"));
        let _ = save(&dir, &journal);
        if let Err(e) = rollback(&dir, &mut journal, &mut log) {
            log(format!("Recovery needs attention: {e:#}"));
        }
    }
    crate::backup_cleanup::finish_operation(state, &_lock, &dir, &mut log);
    result
}
pub fn backups(state: &Path) -> Result<Vec<RootBackupInfo>> {
    Ok(crate::backup_cleanup::list(state)?
        .into_iter()
        .filter(|record| record.kind == crate::backup_cleanup::Kind::Root)
        .map(|record| {
            let instance = platform::read_journal(&record.path.join("root-journal.json"))
                .ok()
                .and_then(|j| j.get("info")?.get("instance")?.as_str().map(str::to_owned))
                .unwrap_or_else(|| "Unknown instance".into());
            RootBackupInfo {
                path: record.path,
                instance,
                created: record.created,
                action: record.title,
                stage: if record.problem.is_some() {
                    format!("{} — needs inspection", record.status)
                } else {
                    record.status
                },
            }
        })
        .collect())
}
pub fn restore_backup(path: &Path, state: &Path, mut log: impl FnMut(String)) -> Result<()> {
    platform::require_admin()?;
    let _lock = platform::lock(state)?;
    let _companion = CompanionLock::take()?;
    let root = platform::absolute(&state.join("root-backups"))?;
    let path = platform::absolute(path)?;
    ensure!(
        path.parent().is_some_and(|p| platform::same_path(p, &root)),
        "Choose a root backup from this app"
    );
    let mut j: Journal =
        serde_json::from_value(platform::read_journal(&path.join("root-journal.json"))?)?;
    ensure!(j.schema == 1, "Unsupported root backup schema");
    ensure!(
        j.stage != "Deleting",
        "This recovery point is being deleted and cannot be restored"
    );
    crate::backup_cleanup::verify_locked(state, &path)?;
    let newer = backups(state)?
        .iter()
        .any(|b| b.created > j.created && b.stage != "Restored");
    ensure!(
        !newer,
        "Restore newer root operations first, because they may share system disks and the player"
    );
    rollback(&path, &mut j, &mut log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_recovery_rejects_corruption_and_preserves_live_ports() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bluestacks.conf");
        let saved = temp.path().join("conf.backup");
        let original = b"bst.instance.Rvc64.enable_root_access=\"0\"\nbst.instance.Pie64.enable_root_access=\"0\"\nbst.instance.Rvc64.adb_port=\"5555\"\n";
        let live = b"bst.instance.Rvc64.enable_root_access=\"1\"\nbst.instance.Pie64.enable_root_access=\"1\"\nbst.instance.Rvc64.adb_port=\"5565\"\n";
        fs::write(&source, live).unwrap();
        fs::write(&saved, live).unwrap(); // Same length; only the checksum detects it.
        let backup = Backup {
            source: source.clone(),
            saved: saved.clone(),
            sha256: platform::hash(original),
            length: original.len() as u64,
        };
        assert!(restore_config(&backup, "Rvc64", false).is_err());
        assert_eq!(fs::read(&source).unwrap(), live);
        fs::write(&saved, original).unwrap();
        restore_config(&backup, "Rvc64", false).unwrap();
        let current = Config::read(&source).unwrap();
        assert_eq!(
            current.get("bst.instance.Rvc64.enable_root_access"),
            Some("0")
        );
        assert_eq!(
            current.get("bst.instance.Pie64.enable_root_access"),
            Some("1")
        );
        assert_eq!(current.get("bst.instance.Rvc64.adb_port"), Some("5565"));
        restore_config(&backup, "Rvc64", true).unwrap();
        assert_eq!(
            Config::read(&source)
                .unwrap()
                .get("bst.instance.Pie64.enable_root_access"),
            Some("0")
        );
        fs::write(&source, b"bst.instance.Rvc64.adb_port=\"5565\"\n").unwrap();
        assert!(restore_config(&backup, "Rvc64", false).is_err());
        assert_eq!(
            fs::read(&source).unwrap(),
            b"bst.instance.Rvc64.adb_port=\"5565\"\n"
        );
    }
}
