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
    sync::Mutex,
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
const REVIEWED_PACKAGES: &[(&str, &str, bool)] = &[
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
];
pub(crate) fn reviewed_package(name: &str) -> bool {
    REVIEWED_PACKAGES
        .iter()
        .any(|(candidate, _, _)| *candidate == name)
}
pub struct Client {
    install: Installation,
    port: u16,
    serial: String,
    player_pid: u32,
    player_birth: u64,
    pub guest: Guest,
    server: Mutex<Child>,
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
fn transport_failure(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_ascii_lowercase();
    // An acknowledged remote failure is not a broken transport, even if the
    // command's own diagnostic contains words such as "not found" or "timeout".
    if message.contains("android command failed (") {
        return false;
    }
    [
        "protocol fault",
        "connection reset",
        "broken pipe",
        "no completion status",
        "device offline",
        "no devices/emulators found",
        "error: closed",
        "timed out",
        "private adb server exited",
        "cannot connect",
        "failed to connect",
    ]
    .iter()
    .any(|part| message.contains(part))
        || (message.contains("error: device") && message.contains("not found"))
}

fn retry_transport<T>(
    mut command: impl FnMut() -> Result<T>,
    mut reconnect: impl FnMut() -> Result<()>,
) -> Result<T> {
    for attempt in 0..4 {
        match command() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < 3 && transport_failure(&error) => {
                // The replacement daemon can itself briefly lose its status
                // channel. Keep that failure within the same bounded retry
                // budget; ownership/identity refusals must still stop at once.
                if let Err(recovery) = reconnect()
                    && !transport_failure(&recovery)
                {
                    return Err(recovery);
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

impl Client {
    /// Persist delayed framework writes before closing a queued player. Nougat
    /// blocks framework power commands, so allow its writeback window then sync.
    pub fn prepare_close(&self) -> Result<()> {
        self.verify_identity()?;
        let sdk = self.idempotent_shell("getprop ro.build.version.sdk")?;
        if sdk.trim().parse::<u32>().is_ok_and(|v| v <= 25) {
            return self.settle_pending_writes();
        }
        self.idempotent_shell("sync")?;
        // Nougat's legacy adbd can fail the PTY shell service. Losing the
        // non-PTY completion marker during shutdown is expected; require
        // positive VM/transport evidence below before accepting it.
        let result = self.shell("svc power shutdown");
        if result.as_ref().is_err_and(|e| {
            let text = format!("{e:#}");
            text.contains("(137)")
                || text.contains("SecurityException")
                || text.contains("Permission Denial")
        }) {
            return self.settle_pending_writes();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if discovery::guest_powered_off(
                &self.install,
                &self.guest.instance,
                self.player_pid,
                self.player_birth,
            ) {
                return Ok(());
            }
            ensure!(
                self.owns_server(),
                "Private ADB server exited before guest shutdown was confirmed"
            );
            match self.raw(&["get-state"], true) {
                Ok(state)
                    if matches!(
                        String::from_utf8_lossy(&state).trim(),
                        "offline" | "unknown"
                    ) =>
                {
                    return Ok(());
                }
                Err(error) => {
                    let message = format!("{error:#}").to_ascii_lowercase();
                    if ["device offline", "not found", "no devices", "error: closed"]
                        .iter()
                        .any(|s| message.contains(s))
                    {
                        return Ok(());
                    }
                }
                _ => {}
            }
            ensure!(
                Instant::now() < deadline,
                "Android did not confirm shutdown; queued player cleanup needs attention: {}",
                result
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|e| format!("{e:#}"))
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    fn settle_pending_writes(&self) -> Result<()> {
        // PackageManager batches restriction writes for 10 seconds; SettingsProvider
        // can defer writes for 2 seconds. Keep the guest alive beyond both windows.
        self.idempotent_shell("sleep 12; sync")?;
        self.verify_identity()
    }
    pub fn reconnect_transport(&self) -> Result<()> {
        let device_port = self
            .serial
            .rsplit_once(':')
            .context("Invalid ADB serial")?
            .1
            .parse::<u16>()?;
        let listeners = network::connections()?
            .into_iter()
            .filter(|r| r.local_port == device_port && r.state == 2)
            .collect::<Vec<_>>();
        ensure!(
            !listeners.is_empty()
                && listeners.iter().all(|r| r.pid == self.player_pid)
                && discovery::processes(&self.install)?
                    .iter()
                    .any(|p| p.pid == self.player_pid
                        && p.start_time == self.player_birth
                        && p.instance.as_deref() == Some(&self.guest.instance)),
            "ADB listener ownership changed; refusing to reconnect to another player"
        );
        if !self.owns_server() {
            self.restart_server()?;
        }
        let _ = self.raw(&["disconnect", &self.serial], false);
        for attempt in 0..4 {
            match self.raw(&["connect", &self.serial], false) {
                Ok(_) => return Ok(()),
                Err(error) if attempt < 3 && transport_failure(&error) => {
                    self.restart_server().with_context(|| {
                        format!("Recover private ADB server after connection failure: {error:#}")
                    })?;
                    std::thread::sleep(Duration::from_millis(150 * (attempt + 1)));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
    /// Only callers with an idempotent command should opt into transport retries.
    pub fn idempotent_shell(&self, cmd: &str) -> Result<String> {
        self.retry_shell(cmd, false)
    }
    fn retry_shell(&self, cmd: &str, verify_identity: bool) -> Result<String> {
        retry_transport(
            || {
                if verify_identity {
                    self.verify_identity()?;
                }
                self.shell(cmd)
            },
            || {
                std::thread::sleep(Duration::from_millis(300));
                self.reconnect_transport()
            },
        )
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
    /// Read-only or explicitly idempotent root commands may recover their transport.
    pub fn idempotent_root(&self, script: &str) -> Result<String> {
        self.idempotent_shell(&format!("su -c {}", quote(script)))
    }
    pub fn verify_identity(&self) -> Result<()> {
        let identity = self.idempotent_shell("settings get secure android_id")?;
        ensure!(
            platform::hash(identity.trim().as_bytes()) == self.guest.identity,
            "Android identity changed during the operation"
        );
        Ok(())
    }
    pub fn connect(install: &Installation, name: &str) -> Result<Self> {
        // Legacy HD-Adb can leave a just-started private daemon with a broken
        // status channel. A constructor failure drops only that owned daemon;
        // retry the read-only connection with a fresh child, never a global server.
        for attempt in 0..3 {
            match Self::connect_once(install, name) {
                Ok(client) => return Ok(client),
                Err(error) if attempt < 2 && transport_failure(&error) => {
                    std::thread::sleep(Duration::from_millis(300 * (attempt + 1)));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
    fn connect_once(install: &Installation, name: &str) -> Result<Self> {
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
        let player_pid = owners[0].pid;
        let player_birth = processes
            .iter()
            .find(|p| p.pid == player_pid)
            .context("ADB player disappeared")?
            .start_time;
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
        let server = spawn_private_server(install, port)?;
        let guest = Guest {
            installation: install.clone(),
            instance: name.into(),
            identity: String::new(),
        };
        let mut client = Self {
            install: install.clone(),
            port,
            serial: format!("127.0.0.1:{device_port}"),
            player_pid,
            player_birth,
            guest,
            server: Mutex::new(server),
            _port_lock: port_lock,
        };
        let start = Instant::now();
        while !client.owns_server() {
            ensure!(
                client
                    .server
                    .get_mut()
                    .map_err(|_| anyhow::anyhow!("Private ADB server lock was poisoned"))?
                    .try_wait()?
                    .is_none(),
                "Private ADB server exited before listening"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "Private ADB server did not become ready"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        client.reconnect_transport()?;
        ensure!(
            client
                .idempotent_shell("getprop sys.boot_completed")?
                .trim()
                == "1",
            "Android is still booting; try again shortly"
        );
        // `pm path` can succeed with empty output for a missing package.
        let launcher = client.idempotent_shell(
            "pm path com.uncube.launcher3 || true; pm path com.bluestacks.launcher || true",
        )?;
        ensure!(
            launcher.contains("package:"),
            "Connected Android device does not contain a recognized BlueStacks launcher"
        );
        let identity = client.idempotent_shell("settings get secure android_id")?;
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
        let Ok(server) = self.server.lock() else {
            return false;
        };
        let Ok(rows) = network::connections() else {
            return false;
        };
        let owners: Vec<_> = rows
            .into_iter()
            .filter(|r| r.local_port == self.port && r.state == 2)
            .collect();
        !owners.is_empty() && owners.iter().all(|r| r.pid == server.id())
    }
    fn restart_server(&self) -> Result<()> {
        {
            let mut server = self
                .server
                .lock()
                .map_err(|_| anyhow::anyhow!("Private ADB server lock was poisoned"))?;
            let _ = server.kill();
            server.wait()?;
            *server = spawn_private_server(&self.install, self.port)?;
        }
        let start = Instant::now();
        while !self.owns_server() {
            ensure!(
                self.server
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Private ADB server lock was poisoned"))?
                    .try_wait()?
                    .is_none(),
                "Private ADB server exited before listening"
            );
            ensure!(
                start.elapsed() < Duration::from_secs(5),
                "Private ADB server did not become ready"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
    fn raw(&self, args: &[&str], device: bool) -> Result<Vec<u8>> {
        ensure!(
            self.owns_server(),
            "Private ADB server exited before the command"
        );
        // A remote host argument prevents adb's implicit daemon auto-start. If
        // our owned child dies, no untracked replacement may be borrowed.
        let mut cmd = vec![
            "-H".into(),
            "127.0.0.1".into(),
            "-P".into(),
            self.port.to_string(),
        ];
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
        let dump = self.idempotent_shell(&format!("dumpsys package {}", quote(name)))?;
        let re = regex::Regex::new(r"User 0: [^\n]*\benabled=(\d+)")?;
        let caps = re
            .captures(&dump)
            .context("Package is absent or its enabled state cannot be read")?;
        let value = caps[1].parse()?;
        ensure!(value <= 4, "Unsupported package state");
        Ok(value)
    }
    pub fn set_package_state(&self, name: &str, state: u8) -> Result<()> {
        ensure!(valid_package(name), "Invalid package");
        let action = match state {
            0 => "default-state",
            1 => "enable",
            2 => "disable",
            3 => "disable-user",
            4 => "disable-until-used",
            _ => bail!("Invalid package state"),
        };
        // Setting an exact state is safe to replay if ADB loses its response.
        // Recheck Android identity before every attempt, then read back the state.
        self.retry_shell(&format!("pm {action} --user 0 {}", quote(name)), true)?;
        ensure!(
            self.package_state(name)? == state,
            "Package state verification failed for {name}"
        );
        Ok(())
    }
    pub fn candidates(&self) -> Result<Vec<Package>> {
        let installed = self.idempotent_shell("pm list packages --user 0")?;
        let mut result = Vec::new();
        for &(name, reason, recommended) in REVIEWED_PACKAGES {
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
        let v = self.idempotent_shell(&format!("settings get global {name}"))?;
        Ok(if v.trim() == "null" {
            None
        } else {
            Some(v.trim().into())
        })
    }
    pub fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        ensure!(ANIMATIONS.contains(&name), "Unsupported Android setting");
        if let Some(v) = value {
            self.retry_shell(&format!("settings put global {name} {}", quote(v)), true)?;
        } else {
            self.retry_shell(&format!("settings delete global {name}"), true)?;
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
                    "-H".into(),
                    "127.0.0.1".into(),
                    "-P".into(),
                    self.port.to_string(),
                    "disconnect".into(),
                    self.serial.clone(),
                ],
                Duration::from_secs(3),
            );
        }
        let server = self.server.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = server.kill();
        let _ = server.wait();
    }
}
fn spawn_private_server(install: &Installation, port: u16) -> Result<Child> {
    Command::new(install.adb())
        .args(["-P", &port.to_string(), "nodaemon", "server"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .context("Start private ADB server")
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
    fn idempotent_write_recovers_when_the_acknowledgement_is_lost() {
        let mut state = 0;
        let mut attempts = 0;
        let mut reconnects = 0;
        retry_transport(
            || {
                attempts += 1;
                state = 3;
                if attempts == 1 {
                    bail!("ADB shell returned no completion status");
                }
                Ok(())
            },
            || {
                reconnects += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((state, attempts, reconnects), (3, 2, 1));
    }
    #[test]
    fn retries_stop_on_identity_or_permission_errors() {
        for message in ["Android identity changed", "Permission denied"] {
            let mut attempts = 0;
            let error = retry_transport::<()>(
                || {
                    attempts += 1;
                    bail!("{message}")
                },
                || panic!("A rejected operation must not reconnect"),
            )
            .unwrap_err();
            assert_eq!(attempts, 1);
            assert_eq!(error.to_string(), message);
        }
    }
    #[test]
    fn retries_are_bounded_and_refuse_changed_listener_ownership() {
        let mut attempts = 0;
        let mut reconnects = 0;
        assert!(
            retry_transport::<()>(
                || {
                    attempts += 1;
                    bail!("Connection reset by peer")
                },
                || {
                    reconnects += 1;
                    Ok(())
                },
            )
            .is_err()
        );
        assert_eq!((attempts, reconnects), (4, 3));
        attempts = 0;
        let error = retry_transport::<()>(
            || {
                attempts += 1;
                bail!("Connection reset by peer")
            },
            || bail!("ADB listener ownership changed"),
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert!(error.to_string().contains("ownership changed"));
    }
    #[test]
    fn transient_reconnect_failure_does_not_abort_an_idempotent_write() {
        let connected = std::cell::Cell::new(false);
        let mut attempts = 0;
        let mut reconnects = 0;
        retry_transport(
            || {
                attempts += 1;
                ensure!(
                    connected.get(),
                    "Private ADB server exited before the command"
                );
                Ok(())
            },
            || {
                reconnects += 1;
                if reconnects == 1 {
                    bail!("error: protocol fault (couldn't read status length): No error");
                }
                connected.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((attempts, reconnects), (3, 2));
    }
    #[test]
    fn transport_retries_are_limited_to_connection_failures() {
        assert!(transport_failure(&anyhow::anyhow!(
            "error: protocol fault (couldn't read status length): No error"
        )));
        assert!(transport_failure(&anyhow::anyhow!(
            "ADB shell returned no completion status"
        )));
        assert!(!transport_failure(&anyhow::anyhow!("Permission denied")));
        assert!(!transport_failure(&anyhow::anyhow!(
            "Android identity changed"
        )));
        assert!(transport_failure(&anyhow::anyhow!(
            "error: device '127.0.0.1:5555' not found"
        )));
        for diagnostic in [
            "Android command failed (127): su: inaccessible or not found",
            "Android command failed (1): connection timed out",
            "Package not found",
        ] {
            assert!(!transport_failure(&anyhow::anyhow!("{diagnostic}")));
        }
    }
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
