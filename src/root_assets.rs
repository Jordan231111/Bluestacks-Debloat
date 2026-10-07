//! Checked payloads imported from the companion project's current distribution.
use crate::platform;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub const APK_HASH: &str = "fac319d2de262fcfff1684e13e1a5c61c486d2a773a7a8ffcfdbfe6f763a7fd4";
pub const BOOTSTRAP_HASH: &str = "7eb6380ee26ce0b68d9f3f23ac04f50e0dfdd49359ef17d1a4978be1795913dd";
const DFS_HASH: &str = "008b6006e766d2591c8c7db7bf6d6a0a4b9cd6116b9a8e2737151828eb577632";
pub const BOOT_ANIMATION: &str = include_str!("../assets/root/bootanim.rc");
pub const BOOT_GATE: &str = include_str!("../assets/root/bsr_boot.sh");
pub const STOCK_BINDMOUNT: &str = include_str!("../assets/root/bindmount.stock");
pub const BOOTSTRAP_BINDMOUNT: &str = include_str!("../assets/root/bindmount.bootstrap");
pub const SYSTEM_CONFIG: &str = "SYSTEMMODE=true\nRECOVERYMODE=false\n";
const APK: &[u8] = include_bytes!("../assets/root/magisk.apk");
const DFS: &[u8] = include_bytes!("../assets/root/debugfs.zip");
const BOOTSTRAP: &[u8] = include_bytes!("../assets/root/bootstrap.gz");

pub struct Assets {
    pub directory: PathBuf,
    pub apk: PathBuf,
    pub debugfs: PathBuf,
    pub bootstrap: PathBuf,
    pub databin: PathBuf,
    pub templates: PathBuf,
}
pub const MAGISK_FILES: &[(&str, &str)] = &[
    ("lib/x86_64/libbusybox.so", "busybox"),
    ("lib/x86_64/libmagisk64.so", "magisk64"),
    ("lib/x86_64/libmagiskboot.so", "magiskboot"),
    ("lib/x86_64/libmagiskinit.so", "magiskinit"),
    ("lib/x86_64/libmagiskpolicy.so", "magiskpolicy"),
    ("lib/x86/libmagisk32.so", "magisk32"),
    ("assets/stub.apk", "stub.apk"),
    ("assets/util_functions.sh", "util_functions.sh"),
    ("assets/boot_patch.sh", "boot_patch.sh"),
    ("assets/addon.d.sh", "addon.d.sh"),
    ("assets/uninstaller.sh", "uninstaller.sh"),
];
pub fn verify_payloads() -> Result<()> {
    for template in [
        BOOT_ANIMATION,
        BOOT_GATE,
        STOCK_BINDMOUNT,
        BOOTSTRAP_BINDMOUNT,
    ] {
        ensure!(
            !template.contains('\r'),
            "Guest root templates must use LF line endings; reimport the checked payloads"
        );
    }
    ensure!(
        platform::hash(APK) == APK_HASH,
        "Embedded Magisk checksum mismatch"
    );
    ensure!(
        platform::hash(DFS) == DFS_HASH,
        "Embedded filesystem-tool checksum mismatch"
    );
    let mut b = Vec::new();
    flate2::read::GzDecoder::new(BOOTSTRAP).read_to_end(&mut b)?;
    ensure!(
        b.len() == 4968 && platform::hash(&b) == BOOTSTRAP_HASH,
        "Embedded bootstrap checksum mismatch"
    );
    Ok(())
}
pub fn extract(directory: &Path) -> Result<Assets> {
    verify_payloads()?;
    ensure!(!directory.exists(), "Root workspace already exists");
    fs::create_dir_all(directory)?;
    let apk = directory.join("magisk.apk");
    platform::atomic_write(&apk, APK)?;
    let mut b = Vec::new();
    flate2::read::GzDecoder::new(BOOTSTRAP).read_to_end(&mut b)?;
    let bootstrap = directory.join("bsr_su");
    platform::atomic_write(&bootstrap, &b)?;
    let dfs_dir = directory.join("debugfs");
    fs::create_dir_all(&dfs_dir)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(DFS))?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let relative = file.enclosed_name().context("Unsafe embedded zip path")?;
        ensure!(
            !file.name().contains(':') && relative.components().count() == 1,
            "Filesystem-tool archive has an unexpected path"
        );
        if file.is_dir() {
            continue;
        }
        ensure!(file.size() <= 32 * 1024 * 1024, "Oversized embedded tool");
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        platform::atomic_write(&dfs_dir.join(relative), &bytes)?;
    }
    let databin = directory.join("databin");
    fs::create_dir_all(&databin)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(APK))?;
    for &(member, name) in MAGISK_FILES {
        let mut file = zip
            .by_name(member)
            .with_context(|| format!("Magisk APK missing {member}"))?;
        ensure!(
            file.size() > 0 && file.size() < 32 * 1024 * 1024,
            "Invalid Magisk member size"
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        platform::atomic_write(&databin.join(name), &bytes)?;
    }
    let templates = directory.join("templates");
    fs::create_dir_all(&templates)?;
    for (name, text) in [
        ("bootanim.rc", BOOT_ANIMATION),
        ("bsr_boot.sh", BOOT_GATE),
        ("config", SYSTEM_CONFIG),
        ("bindmount.bootstrap", BOOTSTRAP_BINDMOUNT),
        ("bindmount.stock", STOCK_BINDMOUNT),
    ] {
        platform::atomic_write(&templates.join(name), text.as_bytes())?;
    }
    let debugfs = dfs_dir.join("debugfs.exe");
    ensure!(debugfs.is_file(), "Embedded debugfs executable missing");
    Ok(Assets {
        directory: directory.into(),
        apk,
        debugfs,
        bootstrap,
        databin,
        templates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_payloads_match_current_companion_distribution() {
        verify_payloads().unwrap();
    }
    #[test]
    fn extracts_complete_payload_without_external_installation() {
        let root = tempfile::tempdir().unwrap();
        let assets = extract(&root.path().join("payload")).unwrap();
        assert!(assets.debugfs.is_file());
        assert_eq!(
            platform::hash(&fs::read(&assets.bootstrap).unwrap()),
            BOOTSTRAP_HASH
        );
        for (_, name) in MAGISK_FILES {
            assert!(assets.databin.join(name).metadata().unwrap().len() > 0);
        }
        assert_eq!(
            fs::read(assets.templates.join("bindmount.stock"))
                .unwrap()
                .len(),
            1339
        );
    }
}
