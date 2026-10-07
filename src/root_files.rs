use crate::{
    platform,
    root_assets::Assets,
    virtual_disk::{Disk, copy_exact},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backup {
    pub source: PathBuf,
    pub saved: PathBuf,
    pub sha256: String,
    pub length: u64,
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut f = File::open(path)?;
    let mut b = vec![0; 4 * 1024 * 1024];
    let mut hash = Sha256::new();
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        hash.update(&b[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn copy_file(source: &Path, dest: &Path, log: &mut impl FnMut(String)) -> Result<String> {
    let parent = dest.parent().context("Backup target has no parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".bsd-copy-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut input = File::open(source)?;
        let size = input.metadata()?.len();
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let mut bucket = 0;
        copy_exact(&mut input, &mut output, size, &mut |done, total| {
            let p = done.saturating_mul(100) / total.max(1);
            if p >= bucket + 10 {
                bucket = p;
                log(format!(
                    "Copying {}: {p}%",
                    source.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        })?;
        output.sync_all()?;
        drop(output);
        log(format!(
            "Verifying the recovery copy of {}",
            source.file_name().unwrap_or_default().to_string_lossy()
        ));
        let hash = hash_file(source)?;
        ensure!(hash_file(&temp)? == hash, "Disk backup verification failed");
        let ok = unsafe {
            if dest.exists() {
                ReplaceFileW(
                    platform::wide(dest).as_ptr(),
                    platform::wide(&temp).as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            } else {
                MoveFileExW(
                    platform::wide(&temp).as_ptr(),
                    platform::wide(dest).as_ptr(),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
        };
        ensure!(
            ok != 0,
            "Replace {}: {}",
            dest.display(),
            std::io::Error::last_os_error()
        );
        Ok(hash)
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn backup(source: &Path, dest: &Path, log: &mut impl FnMut(String)) -> Result<Backup> {
    ensure!(!dest.exists(), "Refusing to overwrite an operation backup");
    let sha256 = copy_file(source, dest, log)?;
    Ok(Backup {
        source: source.into(),
        saved: dest.into(),
        sha256,
        length: source.metadata()?.len(),
    })
}
pub fn restore(b: &Backup, log: &mut impl FnMut(String)) -> Result<()> {
    ensure!(
        b.saved.metadata()?.len() == b.length && hash_file(&b.saved)? == b.sha256,
        "Recovery copy is incomplete or changed: {}",
        b.saved.display()
    );
    ensure!(
        copy_file(&b.saved, &b.source, log)? == b.sha256,
        "Restored file hash differs"
    );
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SystemFile {
    pub path: String,
    pub source: PathBuf,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}
fn dfs_path(path: &Path) -> Result<String> {
    let p = path.to_string_lossy().replace('\\', "/");
    ensure!(
        !p.contains(['"', '\r', '\n']),
        "Unsupported debugfs path characters"
    );
    Ok(format!("\"{p}\""))
}
fn dfs(assets: &Assets, image: &Path, args: &[String]) -> Result<String> {
    let mut command = args.to_vec();
    command.push(image.to_string_lossy().replace('\\', "/"));
    Ok(String::from_utf8_lossy(&platform::run_merged(
        &assets.debugfs,
        &command,
        Duration::from_secs(120),
    )?)
    .into_owned())
}
pub fn edit_system(
    vhd: &Path,
    assets: &Assets,
    files: &[SystemFile],
    remove: &[&str],
    log: &mut impl FnMut(String),
) -> Result<()> {
    let mut disk = Disk::attach(vhd, false)?;
    let region = disk.ext4()?;
    let image = assets
        .directory
        .join(format!("system-{}.img", uuid::Uuid::new_v4()));
    let result = (|| {
        log(format!(
            "Staging the Android system partition ({:.1} GiB)",
            region.length as f64 / 1073741824.0
        ));
        let mut bucket = 0;
        disk.carve(region, &image, |done, total| {
            let p = done * 100 / total;
            if p >= bucket + 10 {
                bucket = p;
                log(format!("Reading system image: {p}%"));
            }
        })?;
        let mut commands = [
            "/android",
            "/android/system",
            "/android/system/etc",
            "/android/system/etc/init",
            "/android/system/etc/init/magisk",
        ]
        .into_iter()
        .map(|p| format!("mkdir {p}"))
        .collect::<Vec<_>>();
        for f in files {
            ensure!(
                f.path.starts_with("/android/system/") && !f.path.contains(['"', '\n', '\r', ' ']),
                "Invalid Android system target"
            );
            let (parent, name) = f.path.rsplit_once('/').context("Missing system parent")?;
            commands.push(format!("cd {parent}"));
            commands.push(format!("rm {name}"));
            commands.push(format!("write {} {name}", dfs_path(&f.source)?));
            commands.push(format!("sif {name} mode 0{:o}", 0o100000 | f.mode));
            commands.push(format!("sif {name} uid {}", f.uid));
            commands.push(format!("sif {name} gid {}", f.gid));
            commands.push(format!("sif {name} links_count 1"));
        }
        for path in remove {
            ensure!(
                path.starts_with("/android/system/"),
                "Invalid removal target"
            );
            commands.push(format!("rm {path}"));
        }
        let cmdfile = assets.directory.join("filesystem-commands.txt");
        platform::atomic_write(&cmdfile, commands.join("\n").as_bytes())?;
        let batch = dfs(
            assets,
            &image,
            &[
                "-w".into(),
                "-f".into(),
                cmdfile.to_string_lossy().replace('\\', "/"),
            ],
        )?;
        for (index, f) in files.iter().enumerate() {
            let stat = dfs(assets, &image, &["-R".into(), format!("stat {}", f.path)])?;
            ensure!(
                stat_matches(&stat, f.source.metadata()?.len(), f.mode, f.uid, f.gid),
                "System-file metadata verification failed: {}\n{stat}\nFilesystem tool output:\n{batch}",
                f.path
            );
            let extracted = assets.directory.join(format!("verify-{index}.bin"));
            if extracted.exists() {
                fs::remove_file(&extracted)?;
            }
            dfs(
                assets,
                &image,
                &[
                    "-R".into(),
                    format!("dump {} {}", f.path, dfs_path(&extracted)?),
                ],
            )?;
            ensure!(
                hash_file(&extracted)? == hash_file(&f.source)?,
                "System-file content verification failed: {}",
                f.path
            );
            fs::remove_file(extracted)?;
        }
        for path in remove {
            let stat = dfs(assets, &image, &["-R".into(), format!("stat {path}")])?;
            ensure!(
                !stat.contains("Inode:") && stat.contains("File not found"),
                "Could not verify bootstrap removal: {path}\n{stat}"
            );
        }
        ensure!(
            image.metadata()?.len() == region.length,
            "Filesystem tool resized its staging image"
        );
        log("System edits verified; committing the staged partition".into());
        let mut bucket = 0;
        disk.write_back(region, &image, |done, total| {
            let p = done * 100 / total;
            if p >= bucket + 10 {
                bucket = p;
                log(format!("Writing system image: {p}%"));
            }
        })?;
        Ok(())
    })();
    let detached = disk.detach();
    if image.exists() {
        let _ = fs::remove_file(&image);
    }
    result?;
    detached?;
    Ok(())
}
pub fn prep_files(a: &Assets) -> Vec<SystemFile> {
    let mut result = Vec::new();
    for name in [
        "magisk32",
        "magisk64",
        "magiskinit",
        "magiskpolicy",
        "stub.apk",
    ] {
        result.push(SystemFile {
            path: format!("/android/system/etc/init/magisk/{name}"),
            source: a.databin.join(name),
            mode: 0o700,
            uid: 0,
            gid: 0,
        });
    }
    for (name, path, mode, uid) in [
        ("config", "/android/system/etc/init/magisk/config", 0o700, 0),
        (
            "bsr_boot.sh",
            "/android/system/etc/init/magisk/bsr_boot.sh",
            0o700,
            0,
        ),
        (
            "bootanim.rc",
            "/android/system/etc/init/bootanim.rc",
            0o664,
            1000,
        ),
        (
            "bindmount.bootstrap",
            "/android/system/bin/bindmount",
            0o755,
            0,
        ),
    ] {
        result.push(SystemFile {
            path: path.into(),
            source: a.templates.join(name),
            mode,
            uid,
            gid: uid,
        });
    }
    for path in ["/android/system/etc/bsr_su", "/android/system/xbin/su"] {
        result.push(SystemFile {
            path: path.into(),
            source: a.bootstrap.clone(),
            mode: 0o6755,
            uid: 0,
            gid: 0,
        });
    }
    result
}
pub fn clean_files(a: &Assets) -> Vec<SystemFile> {
    vec![SystemFile {
        path: "/android/system/bin/bindmount".into(),
        source: a.templates.join("bindmount.stock"),
        mode: 0o775,
        uid: 1000,
        gid: 1000,
    }]
}
fn stat_matches(stat: &str, length: u64, mode: u32, uid: u32, gid: u32) -> bool {
    let mode_ok = regex::Regex::new(r"Mode:\s*([0-7]+)")
        .unwrap()
        .captures(stat)
        .and_then(|c| u32::from_str_radix(&c[1], 8).ok())
        == Some(mode);
    mode_ok
        && stat.contains("Inode:")
        && [
            ("Size", length),
            ("User", uid as u64),
            ("Group", gid as u64),
            ("Links", 1),
        ]
        .iter()
        .all(|(key, v)| {
            regex::Regex::new(&format!(r"\b{key}:\s*{v}\b"))
                .unwrap()
                .is_match(stat)
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_verification_rejects_wrong_owner_or_mode() {
        let stat = "Inode: 12 Type: regular Mode: 06755\nUser: 0 Group: 0 Size: 4968\nLinks: 1";
        assert!(stat_matches(stat, 4968, 0o6755, 0, 0));
        assert!(!stat_matches(stat, 4968, 0o755, 0, 0));
        assert!(!stat_matches(stat, 4968, 0o6755, 1000, 0));
    }
    #[test]
    fn backup_round_trip_checks_corruption() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("source");
        let b = t.path().join("backup");
        fs::write(&a, b"original").unwrap();
        let backup = backup(&a, &b, &mut |_| {}).unwrap();
        fs::write(&a, b"changed").unwrap();
        restore(&backup, &mut |_| {}).unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"original");
        fs::write(&b, b"corrupt").unwrap();
        assert!(restore(&backup, &mut |_| {}).is_err());
    }
}
