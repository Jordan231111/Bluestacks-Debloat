//! Automatic offline-maintenance shutdown, following BluestacksRoot's Kill-BlueStacks.
//! Verify installation ownership and process birth time before terminating anything.
use crate::{
    adb::Client,
    cloud,
    discovery::{self, Installation, Process},
    platform,
};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use sysinfo::{ProcessesToUpdate, System};
use windows_sys::Win32::System::{Environment::ExpandEnvironmentStringsW, Services::*};
use winreg::{RegKey, enums::*};

fn within(path: &Path, root: &Path) -> bool {
    platform::within_directory(path, root)
}

fn vendor_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    (["hd-", "bstk", "bluestacks"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || matches!(name.as_str(), "update.exe" | "crashpad_handler.exe"))
        && name != "bluestacksdebloat.exe"
}

fn roots(install: &Installation, cloud_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![
        platform::absolute(&install.install_dir).unwrap_or_else(|_| install.install_dir.clone()),
    ];
    // A fixture/custom data directory must never make tests stop unrelated cloud apps.
    let registered = discovery::installations()
        .unwrap_or_default()
        .iter()
        .any(|known| platform::same_path(&known.install_dir, &install.install_dir));
    if registered {
        roots.extend(
            cloud::cloud_roots(true, true)
                .into_iter()
                .filter_map(|(path, _)| platform::absolute(&path).ok()),
        );
    }
    for path in cloud_paths {
        if ["BlueStacks X.exe", "BlueStacksServices.exe"]
            .iter()
            .any(|marker| path.join(marker).is_file())
            && let Ok(path) = platform::absolute(path)
        {
            roots.push(path);
        }
    }
    roots.retain(|path| {
        platform::same_path(path, &install.install_dir)
            || (!within(&install.install_dir, path)
                && !within(path, &install.install_dir)
                && !within(&install.data_dir, path)
                && !within(path, &install.data_dir))
    });
    roots
}

fn processes(install: &Installation, roots: &[PathBuf]) -> Result<Vec<Process>> {
    let mut found = discovery::processes(install)?
        .into_iter()
        .map(|p| (p.pid, p))
        .collect::<BTreeMap<_, _>>();
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    for (pid, process) in system.processes() {
        if pid.as_u32() == std::process::id()
            || process.name().eq_ignore_ascii_case("BluestacksDebloat.exe")
        {
            continue;
        }
        if let Some(path) = process.exe()
            && roots.iter().any(|root| within(path, root))
        {
            found.entry(pid.as_u32()).or_insert_with(|| Process {
                pid: pid.as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                path: Some(path.into()),
                instance: None,
                memory_mb: process.memory() / 1024 / 1024,
                start_time: process.start_time(),
            });
        }
    }
    Ok(found.into_values().collect())
}

fn may_stop(process: &Process, roots: &[PathBuf]) -> bool {
    let owned = match &process.path {
        Some(path) => roots.iter().any(|root| within(path, root)),
        None => process.name.eq_ignore_ascii_case("HD-Player.exe") && process.instance.is_some(),
    };
    process.pid != std::process::id() && vendor_name(&process.name) && owned
}

struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

fn service_binary(command: &str) -> Option<PathBuf> {
    let text = command.trim();
    let executable = if let Some(text) = text.strip_prefix('"') {
        text.split_once('"')?.0
    } else {
        let end = text.to_ascii_lowercase().find(".exe")? + 4;
        if !text[end..].is_empty() && !text[end..].starts_with(char::is_whitespace) {
            return None;
        }
        &text[..end]
    };
    let executable = executable.trim_start_matches(r"\??\");
    let wide = platform::wide(executable);
    let len = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), std::ptr::null_mut(), 0) };
    if len == 0 || len > 32768 {
        return None;
    }
    let mut expanded = vec![0u16; len as usize];
    let written = unsafe { ExpandEnvironmentStringsW(wide.as_ptr(), expanded.as_mut_ptr(), len) };
    if written == 0 || written > len {
        return None;
    }
    let path = PathBuf::from(String::from_utf16_lossy(
        &expanded[..written.saturating_sub(1) as usize],
    ));
    path.is_absolute().then_some(path)
}

fn service_status(service: &ServiceHandle) -> Result<SERVICE_STATUS_PROCESS> {
    let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    let ok = unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            &mut status as *mut _ as *mut u8,
            std::mem::size_of_val(&status) as u32,
            &mut needed,
        )
    };
    ensure!(
        ok != 0,
        "Inspect BlueStacks service: {}",
        std::io::Error::last_os_error()
    );
    Ok(status)
}

fn stop_services(roots: &[PathBuf], log: &mut impl FnMut(String)) -> Result<()> {
    let services = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Services", KEY_READ)?;
    let mut names = Vec::new();
    for name in services.enum_keys() {
        let name = name?;
        if !vendor_name(&name) {
            continue;
        }
        let key = services.open_subkey_with_flags(&name, KEY_READ)?;
        let kind: u32 = key.get_value("Type").unwrap_or(0);
        // Kernel drivers do not need to be disabled for config/hosts maintenance.
        if kind & SERVICE_WIN32 == 0 {
            continue;
        }
        let command: String = key.get_value("ImagePath").unwrap_or_default();
        if service_binary(&command).is_some_and(|path| roots.iter().any(|root| within(&path, root)))
        {
            names.push(name);
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    let manager = ServiceHandle(unsafe {
        OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT)
    });
    ensure!(
        !manager.0.is_null(),
        "Open Windows service manager: {}",
        std::io::Error::last_os_error()
    );
    for name in names {
        let service = ServiceHandle(unsafe {
            OpenServiceW(
                manager.0,
                platform::wide(&name).as_ptr(),
                SERVICE_STOP | SERVICE_QUERY_STATUS,
            )
        });
        ensure!(
            !service.0.is_null(),
            "Open BlueStacks service {name}: {}",
            std::io::Error::last_os_error()
        );
        let mut status = service_status(&service)?;
        if status.dwCurrentState == SERVICE_STOPPED {
            continue;
        }
        log(format!("Stopping BlueStacks service: {name}"));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if status.dwCurrentState == SERVICE_STOPPED {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "BlueStacks service {name} did not stop; no offline changes started"
            );
            if !matches!(
                status.dwCurrentState,
                SERVICE_STOP_PENDING | SERVICE_START_PENDING
            ) {
                let mut result: SERVICE_STATUS = unsafe { std::mem::zeroed() };
                if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut result) } == 0 {
                    let error = std::io::Error::last_os_error();
                    ensure!(
                        matches!(error.raw_os_error(), Some(1061 | 1062)),
                        "Stop BlueStacks service {name}: {error}"
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(150));
            status = service_status(&service)?;
        }
    }
    Ok(())
}

pub fn stop(install: &Installation, log: &mut impl FnMut(String)) -> Result<()> {
    stop_with_cloud(install, &[], log)
}

pub fn stop_with_cloud(
    install: &Installation,
    cloud_paths: &[PathBuf],
    log: &mut impl FnMut(String),
) -> Result<()> {
    platform::require_admin()?;
    let roots = roots(install, cloud_paths);
    let initial = processes(install, &roots)?;
    if !initial.is_empty() {
        log(
            "Closing BlueStacks players, managers, ADB and companion processes automatically"
                .into(),
        );
    }
    for process in &initial {
        if may_stop(process, &roots)
            && process.name.eq_ignore_ascii_case("HD-Player.exe")
            && let Some(instance) = &process.instance
            && let Ok(adb) = Client::connect(install, instance)
            && adb.shell("sync").is_ok()
        {
            log(format!("Synced Android writes: {instance}"));
        }
    }
    stop_services(&roots, log)?;
    platform::close_windows(
        &initial
            .iter()
            .filter(|p| may_stop(p, &roots))
            .map(|p| p.pid)
            .collect::<Vec<_>>(),
    );
    if !initial.is_empty() {
        std::thread::sleep(Duration::from_millis(800));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut stable = false;
    loop {
        let remaining = processes(install, &roots)?;
        if remaining.is_empty() {
            if stable {
                log("BlueStacks shutdown verified. Offline changes can continue.".into());
                return Ok(());
            }
            stable = true;
        } else {
            stable = false;
            ensure!(
                Instant::now() < deadline,
                "BlueStacks did not stop automatically; no offline changes started. Remaining: {}",
                remaining
                    .iter()
                    .map(|p| format!("{} ({})", p.name, p.pid))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let mut system = System::new();
            system.refresh_processes(ProcessesToUpdate::All, true);
            for process in remaining {
                // A terminating process can lose its path or acquire incomplete
                // metadata between two snapshots. Wait for it to disappear, without
                // killing an unverified PID or permitting offline writes early.
                if !may_stop(&process, &roots) {
                    continue;
                }
                if let Some(current) = system.process(sysinfo::Pid::from_u32(process.pid))
                    && current.start_time() == process.start_time
                    && current.name().eq_ignore_ascii_case(&process.name)
                    && current
                        .exe()
                        .is_none_or(|path| roots.iter().any(|root| within(path, root)))
                {
                    log(format!("Stopping {} (PID {})", process.name, process.pid));
                    if !current.kill() {
                        let still_running = processes(install, &roots)?
                            .iter()
                            .any(|p| p.pid == process.pid && p.start_time == process.start_time);
                        ensure!(
                            !still_running,
                            "Windows could not stop {} (PID {}); no offline changes started",
                            process.name,
                            process.pid
                        );
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_requires_path_boundary_and_excludes_this_app() {
        let roots = vec![PathBuf::from(r"C:\Program Files\BlueStacks_nxt")];
        let mut process = Process {
            pid: std::process::id().wrapping_add(1),
            name: "BstkSVC.exe".into(),
            path: Some(PathBuf::from(
                r"c:\program files\BLUESTACKS_nxt\BstkSVC.exe",
            )),
            instance: None,
            memory_mb: 0,
            start_time: 1,
        };
        assert!(may_stop(&process, &roots));
        process.name = "HD-Adb.exe".into();
        assert!(may_stop(&process, &roots));
        process.path = Some(PathBuf::from(
            r"C:\Program Files\BlueStacks_nxt-other\HD-Adb.exe",
        ));
        assert!(!may_stop(&process, &roots));
        process.name = "HD-Player.exe".into();
        process.instance = Some("Pie64".into());
        assert!(!may_stop(&process, &roots));
        process.path = Some(PathBuf::from(
            r"C:\Program Files\BlueStacks_nxt\..\Other\HD-Player.exe",
        ));
        assert!(!may_stop(&process, &roots));
        process.instance = None;
        process.path = None;
        process.name = "HD-Player.exe".into();
        assert!(!may_stop(&process, &roots));
        process.instance = Some("Pie64".into());
        assert!(may_stop(&process, &roots));
        process.name = "BluestacksDebloat.exe".into();
        assert!(!may_stop(&process, &roots));
        process.name = "HD-Player.exe".into();
        process.pid = std::process::id();
        assert!(!may_stop(&process, &roots));
    }
    #[test]
    fn service_paths_parse_quotes_spaces_and_nt_prefixes() {
        let expected = PathBuf::from(r"C:\Program Files\BlueStacks_nxt\BstkSVC.exe");
        assert_eq!(
            service_binary(r#""C:\Program Files\BlueStacks_nxt\BstkSVC.exe" --service"#),
            Some(expected.clone())
        );
        assert_eq!(
            service_binary(r"C:\Program Files\BlueStacks_nxt\BstkSVC.exe --service"),
            Some(expected.clone())
        );
        assert_eq!(
            service_binary(r"\??\C:\Program Files\BlueStacks_nxt\BstkSVC.exe"),
            Some(expected)
        );
        assert!(service_binary("BstkSVC.exe").is_none());
        assert!(service_binary(r"C:\Windows\file.exe-not-a-program").is_none());
    }
    #[test]
    fn incomplete_exit_metadata_never_authorizes_termination() {
        let mut process = Process {
            pid: 123,
            name: "HD-Player.exe".into(),
            path: None,
            instance: None,
            memory_mb: 0,
            start_time: 0,
        };
        let roots = vec![PathBuf::from(r"C:\Program Files\BlueStacks_nxt")];
        assert!(!may_stop(&process, &roots));
        process.start_time = 100;
        assert!(!may_stop(&process, &roots));
        process.start_time = 101;
        assert!(!may_stop(&process, &roots));
        process.start_time = 0;
        process.name = "unrelated.exe".into();
        assert!(!may_stop(&process, &roots));
        process.name = "HD-Player.exe".into();
        process.path = Some(PathBuf::from(r"C:\Other\HD-Player.exe"));
        assert!(!may_stop(&process, &roots));
    }
    #[test]
    fn shutdown_stops_owned_service_and_adb_helpers_and_preserves_a_sibling() {
        use std::{
            fs,
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        if !platform::is_admin() {
            return;
        }
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let install_dir = temp.path().join("install");
        let other_dir = temp.path().join("install-other");
        fs::create_dir(&install_dir).unwrap();
        fs::create_dir(&other_dir).unwrap();
        let command = PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/cmd.exe");
        let spawn = |directory: &Path, name: &str| {
            let exe = directory.join(name);
            fs::copy(&command, &exe).unwrap();
            Child(
                Command::new(exe)
                    .args(["/d", "/q", "/k"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(0x08000000)
                    .spawn()
                    .unwrap(),
            )
        };
        let mut service = spawn(&install_dir, "BstkSVC.exe");
        let mut adb = spawn(&install_dir, "HD-Adb.exe");
        let mut unrelated = spawn(&other_dir, "BstkSVC.exe");
        std::thread::sleep(Duration::from_millis(100));
        assert!(service.0.try_wait().unwrap().is_none());
        assert!(adb.0.try_wait().unwrap().is_none());
        assert!(unrelated.0.try_wait().unwrap().is_none());
        let install = Installation {
            install_dir,
            data_dir: temp.path().join("data"),
            version: "fixture".into(),
            source: "fixture".into(),
        };
        let result = stop(&install, &mut |_| {});
        let stopped_service = service.0.try_wait().unwrap().is_some();
        let stopped_adb = adb.0.try_wait().unwrap().is_some();
        let retained_other = unrelated.0.try_wait().unwrap().is_none();
        result.unwrap();
        assert!(stopped_service && stopped_adb && retained_other);
    }
}
