use crate::{
    discovery::{self, Installation},
    network, platform,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    net::TcpListener,
    os::windows::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Guest {
    pub installation: Installation,
    pub instance: String,
    pub identity: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub enabled: u8,
    pub reason: String,
    pub recommended: bool,
}
pub struct Client {
    install: Installation,
    port: u16,
    serial: String,
    pub guest: Guest,
    server: Child,
    _port_lock: File,
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn valid_package(s: &str) -> bool {
    s.contains('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_')
}

impl Client {
    pub fn reconnect_transport(&self) -> Result<()> {
        ensure!(self.owns_server(), "Private ADB server exited");
        let _ = self.raw(&["disconnect", &self.serial], false);
        self.raw(&["connect", &self.serial], false)?;
        Ok(())
    }
    /// Only callers with an idempotent command should opt into transport retries.
    pub fn idempotent_shell(&self, cmd: &str) -> Result<String> {
        let mut last = None;
        for attempt in 0..4 {
            match self.shell(cmd) {
                Ok(s) => return Ok(s),
                Err(e) => {
                    let message = format!("{e:#}").to_ascii_lowercase();
                    if ![
                        "protocol fault",
                        "connection reset",
                        "broken pipe",
                        "no completion status",
                        "device offline",
                        "not found",
                        "error: closed",
                        "timed out",
                    ]
                    .iter()
                    .any(|s| message.contains(s))
                    {
                        return Err(e);
                    }
                    last = Some(e);
                    if attempt < 3 {
                        std::thread::sleep(Duration::from_millis(300));
                        self.reconnect_transport()?;
                    }
                }
            }
        }
        Err(last.unwrap())
    }
    pub fn push_file(&self, source: &Path, target: &str) -> Result<()> {
        ensure!(
            target.starts_with("/data/local/tmp/") && !target.contains(".."),
            "Invalid guest staging target"
        );
        let path = source.to_string_lossy();
        let expected = crate::root_files::hash_file(source)?;
        for attempt in 0..3 {
            let result = self.raw(&["push", &path, target], true);
            if result.is_ok()
                && self
                    .idempotent_shell(&format!("sha256sum {}", quote(target)))
                    .is_ok_and(|s| s.split_whitespace().next() == Some(expected.as_str()))
            {
                return Ok(());
            }
            if attempt < 2 {
                self.reconnect_transport()?;
            }
        }
        bail!("Could not push and verify {}", source.display())
    }
    pub fn install_apk(&self, source: &Path) -> Result<()> {
        let path = source.to_string_lossy();
        let mut last = String::new();
        for attempt in 0..3 {
            match self.raw(&["install", "-r", &path], true) {
                Ok(bytes) => {
                    last = String::from_utf8_lossy(&bytes).into_owned();
                    if last.lines().any(|line| line.trim() == "Success") {
                        return Ok(());
                    }
                    ensure!(
                        !last.contains("Failure ["),
                        "Magisk manager installation failed: {last}"
                    );
                }
                Err(error) => last = format!("{error:#}"),
            }
            if attempt < 2 {
                self.reconnect_transport()?;
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        bail!("Magisk manager installation did not confirm completion: {last}")
    }
    pub fn uninstall_magisk(&self) -> Result<()> {
        let found = self.idempotent_shell("pm path io.github.huskydg.magisk || true")?;
        if found.contains("package:") {
            let bytes = self.raw(&["uninstall", "io.github.huskydg.magisk"], true)?;
            let output = String::from_utf8_lossy(&bytes);
            ensure!(
                output.lines().any(|line| line.trim() == "Success"),
                "Magisk manager uninstall failed: {output}"
            );
        }
        Ok(())
    }
    pub fn root(&self, script: &str) -> Result<String> {
        self.shell(&format!("su -c {}", quote(script)))
    }
    pub fn verify_identity(&self) -> Result<()> {
        let identity = self.shell("settings get secure android_id")?;
        ensure!(
            platform::hash(identity.trim().as_bytes()) == self.guest.identity,
            "Android identity changed during the operation"
        );
        Ok(())
    }
    pub fn connect(install: &Installation, name: &str) -> Result<Self> {
        let conf = crate::config::Config::read(&install.conf())?;
        let instance = discovery::instances(install, &conf)
            .into_iter()
            .find(|i| i.name == name)
            .context("Select a valid instance")?;
        ensure!(
            conf.get("bst.enable_adb_access") == Some("1"),
            "Enable Android Debug Bridge in BlueStacks Settings > Advanced, then restart the selected instance"
        );
        let device_port = instance
            .adb_port
            .context("No ADB port published; start the selected instance first")?;
        let owners: Vec<_> = network::connections()?
            .into_iter()
            .filter(|c| c.local_port == device_port && c.state == 2)
            .collect();
        ensure!(
            !owners.is_empty(),
            "Instance {name} is offline. Start it and wait for the Android home screen"
        );
        let processes = discovery::processes(install)?;
        ensure!(
            owners
                .iter()
                .all(|o| processes.iter().any(|p| p.pid == o.pid
                    && p.name.eq_ignore_ascii_case("HD-Player.exe")
                    && p.instance.as_deref() == Some(name))),
            "ADB port {device_port} belongs to another or unverified instance; refresh after the chosen instance finishes booting"
        );
        let locks = platform::state_dir().join("adb-locks");
        std::fs::create_dir_all(&locks)?;
        let (port, port_lock) = (15037..=15057)
            .find_map(|p| {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .open(locks.join(format!("{p}.lock")))
                    .ok()?;
                TcpListener::bind(("127.0.0.1", p)).ok()?;
                Some((p, file))
            })
            .context("No free private ADB server port (15037–15057)")?;
        // Own the foreground server process handle, rather than borrowing a daemon
        // that a concurrent tool might also use. Drop can only terminate this child.
        let server = Command::new(install.adb())
            .args(["-P", &port.to_string(), "nodaemon", "server"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()
            .context("Start private ADB server")?;
        let guest = Guest {
            installation: install.clone(),
            instance: name.into(),
            identity: String::new(),
        };
        let mut client = Self {
            install: install.clone(),
            port,
            serial: format!("127.0.0.1:{device_port}"),
            guest,
            server,
            _port_lock: port_lock,
        };
        let start = Instant::now();
        while !client.owns_server() {
            ensure!(
                client.server.try_wait()?.is_none(),
                "Private ADB server exited before listening"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "Private ADB server did not become ready"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        client.raw(&["connect", &client.serial], false)?;
        ensure!(
            client.shell("getprop sys.boot_completed")?.trim() == "1",
            "Android is still booting; try again shortly"
        );
        let launcher =
            client.shell("pm path com.uncube.launcher3 || pm path com.bluestacks.launcher")?;
        ensure!(
            launcher.contains("package:"),
            "Connected Android device does not contain a recognized BlueStacks launcher"
        );
        let identity = client.shell("settings get secure android_id")?;
        ensure!(
            !identity.trim().is_empty() && identity.trim() != "null",
            "Cannot identify this Android instance"
        );
        client.guest.identity = platform::hash(identity.trim().as_bytes());
        Ok(client)
    }
    pub fn reconnect(guest: &Guest) -> Result<Self> {
        let client = Self::connect(&guest.installation, &guest.instance)?;
        ensure!(
            client.guest.identity == guest.identity,
            "Android identity changed; refusing to apply a backup or preview to a different instance"
        );
        Ok(client)
    }
    fn owns_server(&self) -> bool {
        let Ok(rows) = network::connections() else {
            return false;
        };
        let owners: Vec<_> = rows
            .into_iter()
            .filter(|r| r.local_port == self.port && r.state == 2)
            .collect();
        !owners.is_empty() && owners.iter().all(|r| r.pid == self.server.id())
    }
    fn raw(&self, args: &[&str], device: bool) -> Result<Vec<u8>> {
        let mut cmd = vec!["-P".into(), self.port.to_string()];
        if device {
            cmd.extend(["-s".into(), self.serial.clone()]);
        }
        cmd.extend(args.iter().map(|a| a.to_string()));
        if args
            .first()
            .is_some_and(|s| ["install", "uninstall", "push"].contains(s))
        {
            platform::run_merged(&self.install.adb(), &cmd, Duration::from_secs(45))
        } else {
            platform::run(&self.install.adb(), &cmd, Duration::from_secs(25))
        }
    }
    pub fn shell(&self, cmd: &str) -> Result<String> {
        let script = format!("{cmd}\nbsd_exit=$?\nprintf '\\n__BSD_EXIT__:%s\\n' \"$bsd_exit\"");
        let bytes = self.raw(&["exec-out", &script], true).with_context(|| {
            format!(
                "Android shell: {}",
                cmd.lines()
                    .next()
                    .unwrap_or("command")
                    .chars()
                    .take(90)
                    .collect::<String>()
            )
        })?;
        let output = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        let (body, code) = output.rsplit_once("\n__BSD_EXIT__:").with_context(|| {
            format!(
                "ADB shell returned no completion status for {}",
                cmd.lines()
                    .next()
                    .unwrap_or("command")
                    .chars()
                    .take(90)
                    .collect::<String>()
            )
        })?;
        ensure!(
            code.trim() == "0",
            "Android command failed ({}): {}",
            code.trim(),
            body.trim()
        );
        Ok(body.trim_end().into())
    }
    pub fn package_state(&self, name: &str) -> Result<u8> {
        ensure!(valid_package(name), "Invalid package name");
        let dump = self.shell(&format!("dumpsys package {}", quote(name)))?;
        let re = regex::Regex::new(r"User 0: [^\n]*\benabled=(\d+)")?;
        let caps = re
            .captures(&dump)
            .context("Package is absent or its enabled state cannot be read")?;
        let value = caps[1].parse()?;
        ensure!(value <= 4, "Unsupported package state");
        Ok(value)
    }
    pub fn set_package_state(&self, name: &str, state: u8) -> Result<()> {
        self.verify_identity()?;
        ensure!(valid_package(name), "Invalid package");
        let action = match state {
            0 => "default-state",
            1 => "enable",
            2 => "disable",
            3 => "disable-user",
            4 => "disable-until-used",
            _ => bail!("Invalid package state"),
        };
        self.shell(&format!("pm {action} --user 0 {}", quote(name)))?;
        ensure!(
            self.package_state(name)? == state,
            "Package state verification failed for {name}"
        );
        Ok(())
    }
    pub fn candidates(&self) -> Result<Vec<Package>> {
        let installed = self.shell("pm list packages --user 0")?;
        let mut result = Vec::new();
        for (name, reason, recommended) in [
            (
                "gg.now.ads.service",
                "BlueStacks ad presentation service",
                true,
            ),
            ("com.uncube.gamevantage", "BlueStacks gameplay ads", true),
            (
                "com.bluestacks.appmart",
                "Legacy BlueStacks App Center",
                true,
            ),
            (
                "com.bluestacks.appfinder",
                "Legacy app recommendations",
                true,
            ),
            ("com.android.egg", "Android Easter egg", false),
            (
                "com.android.printspooler",
                "Android print support (printing stops working)",
                false,
            ),
            ("com.android.traceur", "Android system tracing UI", false),
        ] {
            if installed
                .lines()
                .any(|l| l.trim() == format!("package:{name}"))
            {
                result.push(Package {
                    name: name.into(),
                    enabled: self.package_state(name)?,
                    reason: reason.into(),
                    recommended,
                });
            }
        }
        Ok(result)
    }
    pub fn setting(&self, name: &str) -> Result<Option<String>> {
        ensure!(ANIMATIONS.contains(&name), "Unsupported Android setting");
        let v = self.shell(&format!("settings get global {name}"))?;
        Ok(if v.trim() == "null" {
            None
        } else {
            Some(v.trim().into())
        })
    }
    pub fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        self.verify_identity()?;
        ensure!(ANIMATIONS.contains(&name), "Unsupported Android setting");
        if let Some(v) = value {
            self.shell(&format!("settings put global {name} {}", quote(v)))?;
        } else {
            self.shell(&format!("settings delete global {name}"))?;
        }
        ensure!(
            self.setting(name)?.as_deref() == value,
            "Android setting verification failed"
        );
        Ok(())
    }
    pub fn screenshot(&self, path: &Path) -> Result<()> {
        let bytes = self.raw(&["exec-out", "screencap", "-p"], true)?;
        ensure!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "ADB returned an invalid screenshot"
        );
        platform::atomic_write(path, &bytes)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if self.owns_server() {
            let _ = platform::run(
                &self.install.adb(),
                &[
                    "-P".into(),
                    self.port.to_string(),
                    "disconnect".into(),
                    self.serial.clone(),
                ],
                Duration::from_secs(3),
            );
        }
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}
pub const ANIMATIONS: &[&str] = &[
    "window_animation_scale",
    "transition_animation_scale",
    "animator_duration_scale",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn package_validation_blocks_shell_injection() {
        assert!(!valid_package("a.b; reboot"));
        assert!(!valid_package("a.b\n"));
        assert!(valid_package("gg.now.ads.service"));
    }
    #[test]
    fn quote_handles_apostrophes() {
        assert_eq!(quote("a'b"), "'a'\\''b'");
    }
}
