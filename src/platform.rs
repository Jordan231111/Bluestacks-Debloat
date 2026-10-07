use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .trim_end_matches(['\\', '/'])
        .eq_ignore_ascii_case(
            b.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .trim_end_matches(['\\', '/']),
        )
}
pub fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(fs::canonicalize(path)?
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .into())
}

pub fn run(program: &Path, args: &[String], timeout: Duration) -> Result<Vec<u8>> {
    run_impl(program, args, timeout, false)
}
pub fn run_merged(program: &Path, args: &[String], timeout: Duration) -> Result<Vec<u8>> {
    run_impl(program, args, timeout, true)
}
fn run_impl(program: &Path, args: &[String], timeout: Duration, merge: bool) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x08000000)
        .spawn()
        .with_context(|| format!("Start {}", program.display()))?;
    fn reader(
        mut pipe: impl Read + Send + 'static,
    ) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let mut buf = [0; 8192];
            let result = (|| {
                loop {
                    let n = pipe.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    if output.len() + n <= 32 * 1024 * 1024 {
                        output.extend_from_slice(&buf[..n]);
                    } else {
                        return Err(std::io::Error::other("Child output exceeds 32 MiB"));
                    }
                }
                Ok(output)
            })();
            let _ = tx.send(result);
        });
        rx
    }
    let stdout = reader(child.stdout.take().unwrap());
    let stderr = reader(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} timed out after {}s",
                program.display(),
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let mut out = stdout
        .recv_timeout(Duration::from_secs(3))
        .context("Child kept stdout open")??;
    let err = stderr
        .recv_timeout(Duration::from_secs(3))
        .context("Child kept stderr open")??;
    ensure!(
        status.success(),
        "{} failed ({}): {} {}",
        program.display(),
        status,
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    if merge {
        out.extend_from_slice(b"\n");
        out.extend_from_slice(&err);
    }
    Ok(out)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("File has no parent")?;
    let temp = parent.join(format!(".bsd-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // ReplaceFile preserves the destination's DACL, streams and other metadata.
        let ok = unsafe {
            if path.exists() {
                ReplaceFileW(
                    wide(path).as_ptr(),
                    wide(&temp).as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            } else {
                MoveFileExW(
                    wide(&temp).as_ptr(),
                    wide(path).as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
        };
        ensure!(
            ok != 0,
            "Replace {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        );
        ensure!(
            fs::read(path)? == bytes,
            "Read-back verification failed: {}",
            path.display()
        );
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("BluestacksDebloat")
}
pub fn lock(root: &Path) -> Result<File> {
    fs::create_dir_all(root)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(root.join("operation.lock"))
        .context("Another BlueStacks Debloat operation is running")
}
pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}
pub fn require_admin() -> Result<()> {
    ensure!(
        is_admin(),
        "Use 'Restart as administrator' before applying host changes"
    );
    Ok(())
}
pub fn elevate() -> Result<()> {
    let exe = std::env::current_exe()?;
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("runas").as_ptr(),
            wide(&exe).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(
        result as isize > 32,
        "Administrator restart was cancelled or failed"
    );
    Ok(())
}
pub fn open_folder(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(result as isize > 32, "Could not open {}", path.display());
    Ok(())
}

pub fn close_windows(pids: &[u32]) {
    unsafe extern "system" fn callback(hwnd: HWND, param: LPARAM) -> i32 {
        let pids = unsafe { &*(param as *const Vec<u32>) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pids.contains(&pid)
            && unsafe { IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() }
        {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    let owned = pids.to_vec();
    unsafe {
        EnumWindows(Some(callback), &owned as *const _ as isize);
    }
}
