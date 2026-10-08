use crate::{
    adb::{Client, Guest},
    config::{Config, Edit},
    discovery::Installation,
    platform,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};
use winreg::{RegKey, RegValue, enums::*};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryPath {
    pub machine: bool,
    pub view32: bool,
    pub key: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Target {
    Config {
        path: PathBuf,
        edits: Vec<Edit>,
    },
    File {
        path: PathBuf,
    },
    Registry(RegistryPath),
    RegistryTree {
        machine: bool,
        view32: bool,
        key: String,
    },
    Package {
        name: String,
    },
    Setting {
        name: String,
    },
    RootModule,
    Directory {
        path: PathBuf,
        stash: PathBuf,
        fingerprint: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Value {
    Missing,
    Bytes(Vec<u8>),
    Registry { kind: u32, bytes: Vec<u8> },
    Number(u8),
    Text(Option<String>),
    Present(bool),
}
#[derive(Clone, Debug)]
pub struct Operation {
    pub label: String,
    pub target: Target,
    pub before: Value,
    pub after: Value,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub installation: Installation,
    pub guest: Option<Guest>,
    pub title: String,
    pub operations: Vec<Operation>,
    pub notes: Vec<String>,
}
impl Plan {
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"title":self.title,"installation":self.installation,"instance":self.guest.as_ref().map(|g|&g.instance),"notes":self.notes,"changes":self.operations.iter().map(|o|serde_json::json!({"label":o.label,"target":o.target,"before":describe(&o.before),"after":describe(&o.after)})).collect::<Vec<_>>()})
    }
}
fn describe(v: &Value) -> String {
    match v {
        Value::Bytes(b) => format!("{} bytes; SHA-256 {}", b.len(), platform::hash(b)),
        v => format!("{v:?}"),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Saved {
    Inline(Value),
    Blob { name: String, sha256: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    label: String,
    target: Target,
    before: Saved,
    after: Saved,
    status: String,
    error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub created: String,
    pub status: String,
    pub installation: Installation,
    pub guest: Option<Guest>,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub title: String,
    pub created: String,
    pub status: String,
}

pub struct Backend {
    pub guest: Option<Client>,
}
impl Backend {
    pub fn new(guest: Option<&Guest>) -> Result<Self> {
        Ok(Self {
            guest: guest.map(Client::reconnect).transpose()?,
        })
    }
    fn adb(&self) -> Result<&Client> {
        self.guest
            .as_ref()
            .context("This backup requires its Android instance running with ADB enabled")
    }
    pub fn read(&self, t: &Target) -> Result<Value> {
        match t {
            Target::Config { path, .. } | Target::File { path } => match fs::read(path) {
                Ok(b) => Ok(Value::Bytes(b)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Missing),
                Err(e) => Err(e.into()),
            },
            Target::Registry(p) => read_registry(p),
            Target::RegistryTree {
                machine,
                view32,
                key,
            } => read_tree(*machine, *view32, key),
            Target::Package { name } => Ok(Value::Number(self.adb()?.package_state(name)?)),
            Target::Setting { name } => Ok(Value::Text(self.adb()?.setting(name)?)),
            Target::RootModule => crate::rooted::read(self.adb()?),
            Target::Directory {
                path,
                stash,
                fingerprint,
            } => {
                ensure!(
                    !(path.exists() && stash.exists()),
                    "Both active and backup directories exist; manual conflict resolution needed"
                );
                let exists = path.exists();
                let present = if exists { path } else { stash };
                ensure!(
                    present.is_dir(),
                    "Neither active nor backup directory exists: {}",
                    path.display()
                );
                ensure!(
                    &tree_hash(present)? == fingerprint,
                    "Directory contents changed since preview/backup: {}",
                    present.display()
                );
                Ok(Value::Present(exists))
            }
        }
    }
    fn write(&self, t: &Target, v: &Value) -> Result<()> {
        match t {
            Target::Config { path, .. } | Target::File { path } => match v {
                Value::Bytes(b) => platform::atomic_write(path, b)?,
                Value::Missing => {
                    if path.exists() {
                        fs::remove_file(path)?;
                    }
                }
                _ => bail!("Invalid file state"),
            },
            Target::Registry(p) => write_registry(p, v)?,
            Target::RegistryTree {
                machine,
                view32,
                key,
            } => write_tree(*machine, *view32, key, v)?,
            Target::Package { name } => {
                let Value::Number(n) = v else {
                    bail!("Invalid package state")
                };
                self.adb()?.set_package_state(name, *n)?;
            }
            Target::Setting { name } => {
                let Value::Text(v) = v else {
                    bail!("Invalid setting state")
                };
                self.adb()?.set_setting(name, v.as_deref())?;
            }
            Target::RootModule => crate::rooted::write(self.adb()?, v)?,
            Target::Directory {
                path,
                stash,
                fingerprint,
            } => {
                let Value::Present(active) = v else {
                    bail!("Invalid directory state")
                };
                let (from, to) = if *active {
                    (stash, path)
                } else {
                    (path, stash)
                };
                ensure!(
                    !to.exists() && from.is_dir(),
                    "Directory move conflicts with an existing destination"
                );
                ensure!(
                    &tree_hash(from)? == fingerprint,
                    "Directory changed since backup"
                );
                // Source and destination are fixed, absolute paths on the same volume.
                ensure!(
                    from.is_absolute() && to.is_absolute(),
                    "Refusing relative directory move"
                );
                fs::create_dir_all(to.parent().context("Missing destination parent")?)?;
                fs::rename(from, to).with_context(|| {
                    format!(
                        "Move {} to {}; a file is still in use after automatic shutdown",
                        from.display(),
                        to.display()
                    )
                })?;
            }
        }
        Ok(())
    }
}
fn hive(machine: bool) -> RegKey {
    RegKey::predef(if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    })
}
fn view(view32: bool) -> u32 {
    if view32 {
        KEY_WOW64_32KEY
    } else {
        KEY_WOW64_64KEY
    }
}
pub fn read_registry(p: &RegistryPath) -> Result<Value> {
    let key = match hive(p.machine).open_subkey_with_flags(&p.key, KEY_READ | view(p.view32)) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Missing),
        Err(e) => return Err(e.into()),
    };
    match key.get_raw_value(&p.name) {
        Ok(v) => Ok(Value::Registry {
            kind: v.vtype as u32,
            bytes: v.bytes,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Missing),
        Err(e) => Err(e.into()),
    }
}
fn regvalue(kind: u32, bytes: Vec<u8>) -> Result<RegValue> {
    let vtype = match kind {
        0 => REG_NONE,
        1 => REG_SZ,
        2 => REG_EXPAND_SZ,
        3 => REG_BINARY,
        4 => REG_DWORD,
        5 => REG_DWORD_BIG_ENDIAN,
        6 => REG_LINK,
        7 => REG_MULTI_SZ,
        8 => REG_RESOURCE_LIST,
        9 => REG_FULL_RESOURCE_DESCRIPTOR,
        10 => REG_RESOURCE_REQUIREMENTS_LIST,
        11 => REG_QWORD,
        _ => bail!("Unsupported registry type {kind}"),
    };
    Ok(RegValue { vtype, bytes })
}
pub fn string_value(s: &str) -> Value {
    Value::Registry {
        kind: 1,
        bytes: s
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect(),
    }
}
fn write_registry(p: &RegistryPath, v: &Value) -> Result<()> {
    match v {
        Value::Missing => {
            if let Ok(k) =
                hive(p.machine).open_subkey_with_flags(&p.key, KEY_SET_VALUE | view(p.view32))
            {
                match k.delete_value(&p.name) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Value::Registry { kind, bytes } => {
            let (k, _) =
                hive(p.machine).create_subkey_with_flags(&p.key, KEY_WRITE | view(p.view32))?;
            k.set_raw_value(&p.name, &regvalue(*kind, bytes.clone())?)?;
        }
        _ => bail!("Invalid registry value"),
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Tree {
    values: Vec<(String, u32, Vec<u8>)>,
    children: Vec<(String, Tree)>,
}
fn capture_tree(k: &RegKey) -> Result<Tree> {
    let mut values = k
        .enum_values()
        .map(|v| v.map(|(n, v)| (n, v.vtype as u32, v.bytes)))
        .collect::<std::io::Result<Vec<_>>>()?;
    values.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names = k.enum_keys().collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    let mut children = Vec::new();
    for name in names {
        children.push((name.clone(), capture_tree(&k.open_subkey(name)?)?));
    }
    Ok(Tree { values, children })
}
pub fn read_tree(machine: bool, view32: bool, path: &str) -> Result<Value> {
    match hive(machine).open_subkey_with_flags(path, KEY_READ | view(view32)) {
        Ok(k) => Ok(Value::Bytes(serde_json::to_vec(&capture_tree(&k)?)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Missing),
        Err(e) => Err(e.into()),
    }
}
fn restore_tree(k: &RegKey, t: &Tree) -> Result<()> {
    for (n, kind, b) in &t.values {
        k.set_raw_value(n, &regvalue(*kind, b.clone())?)?;
    }
    for (n, c) in &t.children {
        restore_tree(&k.create_subkey(n)?.0, c)?;
    }
    Ok(())
}
fn write_tree(machine: bool, view32: bool, path: &str, v: &Value) -> Result<()> {
    ensure!(
        path.starts_with("SOFTWARE\\") || path.starts_with("Software\\"),
        "Registry tree outside SOFTWARE"
    );
    let (parent, leaf) = path
        .rsplit_once('\\')
        .context("Registry path has no parent")?;
    let (root, _) =
        hive(machine).create_subkey_with_flags(parent, KEY_ALL_ACCESS | view(view32))?;
    if root.open_subkey(leaf).is_ok() {
        root.delete_subkey_all(leaf)?;
    }
    match v {
        Value::Missing => {}
        Value::Bytes(b) => restore_tree(&root.create_subkey(leaf)?.0, &serde_json::from_slice(b)?)?,
        _ => bail!("Invalid registry tree"),
    };
    Ok(())
}

pub fn tree_hash(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut paths = vec![path.to_owned()];
    let base = platform::absolute(path)?;
    while let Some(dir) = paths.pop() {
        let meta = fs::symlink_metadata(&dir)?;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "Reparse point in removal tree: {}",
            dir.display()
        );
        let mut entries = fs::read_dir(&dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let m = fs::symlink_metadata(&p)?;
            ensure!(
                m.file_attributes() & 0x400 == 0,
                "Reparse point in removal tree: {}",
                p.display()
            );
            let relative = p.strip_prefix(&base).or_else(|_| p.strip_prefix(path))?;
            hash.update(relative.to_string_lossy().as_bytes());
            hash.update([0]);
            if m.is_dir() {
                paths.push(p);
            } else {
                let mut f = fs::File::open(p)?;
                let mut b = [0u8; 65536];
                loop {
                    let n = f.read(&mut b)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&b[..n]);
                }
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn save_value(dir: &Path, stem: &str, value: &Value) -> Result<Saved> {
    if let Value::Bytes(bytes) = value {
        let name = format!("{stem}.bin");
        platform::atomic_write(&dir.join(&name), bytes)?;
        Ok(Saved::Blob {
            name,
            sha256: platform::hash(bytes),
        })
    } else {
        Ok(Saved::Inline(value.clone()))
    }
}
fn load_value(dir: &Path, value: &Saved) -> Result<Value> {
    match value {
        Saved::Inline(v) => Ok(v.clone()),
        Saved::Blob { name, sha256 } => {
            ensure!(
                Path::new(name).components().count() == 1 && !name.contains(['\\', '/']),
                "Invalid backup filename"
            );
            let b = fs::read(dir.join(name))?;
            ensure!(
                &platform::hash(&b) == sha256,
                "Backup integrity check failed for {name}"
            );
            Ok(Value::Bytes(b))
        }
    }
}
fn save(dir: &Path, journal: &Journal) -> Result<()> {
    platform::write_journal(&dir.join("journal.json"), journal)
}

pub fn apply(plan: Plan, root: &Path, mut log: impl FnMut(String)) -> Result<PathBuf> {
    ensure!(!plan.operations.is_empty(), "No changes needed");
    let mut targets = std::collections::BTreeSet::new();
    for op in &plan.operations {
        let key = serde_json::to_string(&op.target)?
            .to_ascii_lowercase()
            .replace('/', "\\");
        ensure!(
            targets.insert(key),
            "Duplicate operation target: {}",
            op.label
        );
    }
    let _lock = platform::lock(root)?;
    if plan.guest.is_none() {
        platform::require_admin()?;
        let cloud_paths = plan
            .operations
            .iter()
            .filter_map(|op| match &op.target {
                Target::Directory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        crate::shutdown::stop_with_cloud(&plan.installation, &cloud_paths, &mut log)?;
    }
    let backend = Backend::new(plan.guest.as_ref())?;
    let id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let dir = root.join("backups").join(&id);
    fs::create_dir_all(&dir)?;
    let mut journal = Journal {
        schema: 1,
        id,
        title: plan.title,
        created: chrono::Utc::now().to_rfc3339(),
        status: "preparing".into(),
        installation: plan.installation,
        guest: plan.guest,
        entries: Vec::new(),
    };
    // Prepare every backup before the first mutation. Stale previews fail here.
    for (i, mut op) in plan.operations.into_iter().enumerate() {
        // Running cloud apps can update logs/databases during review. Snapshot the
        // current directory only after automatic shutdown and before the first write.
        if let Target::Directory {
            path, fingerprint, ..
        } = &mut op.target
            && fingerprint.is_empty()
        {
            *fingerprint = tree_hash(path)?;
        }
        let current = backend.read(&op.target)?;
        if let Target::Config { edits, .. } = &op.target {
            let Value::Bytes(ref bytes) = current else {
                bail!("Config disappeared")
            };
            op.after =
                Value::Bytes(Config::parse(String::from_utf8(bytes.clone())?)?.edit(edits, false)?);
            op.before = current;
        } else {
            ensure!(
                current == op.before,
                "{} changed since preview; preview again",
                op.label
            );
        }
        journal.entries.push(Entry {
            label: op.label,
            target: op.target,
            before: save_value(&dir, &format!("{i:03}-before"), &op.before)?,
            after: save_value(&dir, &format!("{i:03}-after"), &op.after)?,
            status: "prepared".into(),
            error: None,
        });
    }
    journal.status = "applying".into();
    save(&dir, &journal)?;
    for i in 0..journal.entries.len() {
        let result = (|| -> Result<()> {
            let e = &journal.entries[i];
            let before = load_value(&dir, &e.before)?;
            let after = load_value(&dir, &e.after)?;
            ensure!(
                backend.read(&e.target)? == before,
                "{} changed during apply",
                e.label
            );
            journal.entries[i].status = "applying".into();
            save(&dir, &journal)?;
            let e = &journal.entries[i];
            log(format!("Applying: {}", e.label));
            backend.write(&e.target, &after)?;
            ensure!(
                backend.read(&e.target)? == after,
                "Verification failed: {}",
                e.label
            );
            journal.entries[i].status = "applied".into();
            save(&dir, &journal)?;
            Ok(())
        })();
        if let Err(error) = result {
            journal.entries[i].error = Some(format!("{error:#}"));
            journal.status = "failed".into();
            let _ = save(&dir, &journal);
            log(format!(
                "Apply failed. Restoring completed changes: {error:#}"
            ));
            let rollback = restore_entries(&dir, &mut journal, &backend, &mut log);
            if rollback.is_ok() {
                crate::backup_cleanup::finish_operation(root, &_lock, &dir, &mut log);
            }
            bail!(
                "Apply failed: {error:#}. Rollback: {}. Backup: {}",
                match rollback {
                    Ok(()) => "complete".into(),
                    Err(e) => format!("needs attention: {e:#}"),
                },
                dir.display()
            );
        }
    }
    journal.status = "applied".into();
    if let Err(error) = save(&dir, &journal) {
        let result = restore_entries(&dir, &mut journal, &backend, &mut log);
        if result.is_ok() {
            crate::backup_cleanup::finish_operation(root, &_lock, &dir, &mut log);
        }
        bail!(
            "Could not finalize journal: {error:#}. Rollback: {result:?}. Backup: {}",
            dir.display()
        );
    }
    log(format!("Verified. Backup: {}", dir.display()));
    crate::backup_cleanup::finish_operation(root, &_lock, &dir, &mut log);
    Ok(dir)
}
fn restore_entries(
    dir: &Path,
    journal: &mut Journal,
    backend: &Backend,
    log: &mut impl FnMut(String),
) -> Result<()> {
    let mut failures = Vec::new();
    for i in (0..journal.entries.len()).rev() {
        if matches!(journal.entries[i].status.as_str(), "prepared" | "restored") {
            continue;
        }
        let result = (|| -> Result<()> {
            let e = &journal.entries[i];
            let before = load_value(dir, &e.before)?;
            let after = load_value(dir, &e.after)?;
            let current = backend.read(&e.target)?;
            if current == before {
                return Ok(());
            }
            let restored = if let (Target::Config { edits, .. }, Value::Bytes(current)) =
                (&e.target, &current)
            {
                Value::Bytes(Config::parse(String::from_utf8(current.clone())?)?.edit(edits, true)?)
            } else {
                ensure!(
                    current == after,
                    "{} was modified after this operation; keeping the newer value",
                    e.label
                );
                before
            };
            log(format!("Restoring: {}", e.label));
            backend.write(&e.target, &restored)?;
            ensure!(
                backend.read(&e.target)? == restored,
                "Restore verification failed: {}",
                e.label
            );
            Ok(())
        })();
        match result {
            Ok(()) => {
                journal.entries[i].status = "restored".into();
                journal.entries[i].error = None;
            }
            Err(e) => {
                failures.push(format!("{}: {e:#}", journal.entries[i].label));
                journal.entries[i].error = Some(format!("{e:#}"));
            }
        }
        if let Err(error) = save(dir, journal) {
            failures.push(format!("Could not save recovery progress: {error:#}"));
        }
    }
    journal.status = if failures.is_empty() {
        "restored"
    } else {
        "restore_incomplete"
    }
    .into();
    if let Err(error) = save(dir, journal) {
        failures.push(format!("Could not finalize recovery journal: {error:#}"));
    }
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
pub fn restore(dir: &Path, root: &Path, mut log: impl FnMut(String)) -> Result<()> {
    let _lock = platform::lock(root)?;
    let base = platform::absolute(&root.join("backups"))?;
    let actual = platform::absolute(dir)?;
    ensure!(
        actual
            .parent()
            .is_some_and(|p| platform::same_path(p, &base)),
        "Select a backup created in this application's backup folder"
    );
    let mut journal: Journal =
        serde_json::from_value(platform::read_journal(&dir.join("journal.json"))?)?;
    ensure!(journal.schema == 1, "Unsupported backup version");
    ensure!(
        journal.status != "Deleting",
        "This recovery point is being deleted and cannot be restored"
    );
    crate::backup_cleanup::verify_locked(root, dir)?;
    if journal.guest.is_none() {
        platform::require_admin()?;
        let cloud_paths = journal
            .entries
            .iter()
            .filter_map(|entry| match &entry.target {
                Target::Directory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        crate::shutdown::stop_with_cloud(&journal.installation, &cloud_paths, &mut log)?;
    }
    let backend = Backend::new(journal.guest.as_ref())?;
    restore_entries(dir, &mut journal, &backend, &mut log)
}
pub fn backups(root: &Path) -> Result<Vec<BackupInfo>> {
    Ok(crate::backup_cleanup::list(root)?
        .into_iter()
        .filter(|record| record.kind == crate::backup_cleanup::Kind::Debloat)
        .map(|record| BackupInfo {
            path: record.path,
            title: record.title,
            created: record.created,
            status: if record.problem.is_some() {
                format!("{} — needs inspection", record.status)
            } else {
                record.status
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_replacement_verifies_and_replaces_existing() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config");
        platform::atomic_write(&p, b"first").unwrap();
        platform::atomic_write(&p, b"second").unwrap();
        assert_eq!(fs::read(p).unwrap(), b"second");
    }
    #[test]
    fn atomic_replacement_preserves_readonly_system_hidden_attributes_and_streams() {
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetFileSecurityW};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
            FILE_ATTRIBUTE_SYSTEM, SetFileAttributesW,
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hosts");
        fs::write(&path, b"original").unwrap();
        let stream = PathBuf::from(format!("{}:owned-test-stream", path.display()));
        fs::write(&stream, b"preserve alternate stream").unwrap();
        let read_acl = || {
            let mut needed = 0;
            unsafe {
                GetFileSecurityW(
                    platform::wide(&path).as_ptr(),
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    0,
                    &mut needed,
                );
            }
            let mut bytes = vec![0u8; needed as usize];
            assert_ne!(
                unsafe {
                    GetFileSecurityW(
                        platform::wide(&path).as_ptr(),
                        DACL_SECURITY_INFORMATION,
                        bytes.as_mut_ptr().cast(),
                        needed,
                        &mut needed,
                    )
                },
                0
            );
            // Newer Windows can append inherited copies of existing ACEs and
            // mark the descriptor auto-inherited during ReplaceFile. Compare
            // principals, rights, ACE order and DACL protection, not that encoding.
            let control = u16::from_le_bytes(bytes[2..4].try_into().unwrap()) & 0x1004;
            let acl = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
            assert!(acl >= 20 && acl + 8 <= bytes.len());
            let count = u16::from_le_bytes(bytes[acl + 4..acl + 6].try_into().unwrap());
            let mut entries = Vec::new();
            let mut at = acl + 8;
            for _ in 0..count {
                let size = u16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap()) as usize;
                let mut entry = bytes[at..at + size].to_vec();
                entry[1] &= !0x10; // INHERITED_ACE does not change a file ACE's rights.
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
                at += size;
            }
            (control, entries)
        };
        let original_acl = read_acl();
        let attributes = FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_SYSTEM;
        assert_ne!(
            unsafe { SetFileAttributesW(platform::wide(&path).as_ptr(), attributes) },
            0
        );
        let result = platform::atomic_write(&path, b"updated");
        let actual_attributes = fs::metadata(&path).unwrap().file_attributes();
        // Release the read-only attribute so the disposable test directory can be removed.
        unsafe {
            SetFileAttributesW(platform::wide(&path).as_ptr(), FILE_ATTRIBUTE_ARCHIVE);
        }
        result.unwrap();
        assert_eq!(actual_attributes, attributes);
        assert_eq!(fs::read(&path).unwrap(), b"updated");
        assert_eq!(fs::read(stream).unwrap(), b"preserve alternate stream");
        assert_eq!(read_acl(), original_acl);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    #[test]
    fn failed_replacement_restores_readonly_attributes_and_original_bytes() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
            FILE_ATTRIBUTE_SYSTEM, SetFileAttributesW,
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hosts");
        fs::write(&path, b"original").unwrap();
        let attributes = FILE_ATTRIBUTE_ARCHIVE
            | FILE_ATTRIBUTE_READONLY
            | FILE_ATTRIBUTE_HIDDEN
            | FILE_ATTRIBUTE_SYSTEM;
        assert_ne!(
            unsafe { SetFileAttributesW(platform::wide(&path).as_ptr(), attributes) },
            0
        );
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let result = platform::atomic_write(&path, b"updated");
        let after = fs::metadata(&path).unwrap().file_attributes();
        drop(held);
        unsafe {
            SetFileAttributesW(platform::wide(&path).as_ptr(), FILE_ATTRIBUTE_ARCHIVE);
        }
        assert!(result.is_err());
        assert_eq!(after, attributes);
        assert_eq!(fs::read(path).unwrap(), b"original");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    #[test]
    fn deferred_cloud_snapshot_captures_files_after_review_and_restores_them() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cloud");
        let stash = temp
            .path()
            .join(".BlueStacksDebloat-backups")
            .join(uuid::Uuid::new_v4().to_string())
            .join("cloud");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("cloud.exe"), b"cloud app").unwrap();
        let plan = Plan {
            installation: Installation {
                install_dir: temp.path().join("install"),
                data_dir: temp.path().into(),
                version: "fixture".into(),
                source: "fixture".into(),
            },
            guest: None,
            title: "Cloud snapshot fixture".into(),
            notes: vec![],
            operations: vec![Operation {
                label: "Remove cloud fixture".into(),
                target: Target::Directory {
                    path: path.clone(),
                    stash: stash.clone(),
                    fingerprint: String::new(),
                },
                before: Value::Present(true),
                after: Value::Present(false),
            }],
        };
        fs::write(path.join("runtime.log"), b"written after review").unwrap();
        let backup = apply(plan, temp.path(), |_| {}).unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::read(stash.join("runtime.log")).unwrap(),
            b"written after review"
        );
        restore(&backup, temp.path(), |_| {}).unwrap();
        assert!(path.join("cloud.exe").exists());
        assert_eq!(
            fs::read(path.join("runtime.log")).unwrap(),
            b"written after review"
        );
    }
    #[test]
    fn backup_corruption_is_rejected() {
        let t = tempfile::tempdir().unwrap();
        let saved = save_value(t.path(), "before", &Value::Bytes(b"original".to_vec())).unwrap();
        fs::write(t.path().join("before.bin"), b"wrong").unwrap();
        assert!(load_value(t.path(), &saved).is_err());
    }
    #[test]
    fn restore_merges_config_and_retries_safely() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config");
        fs::write(&p, b"ad=0\nport=5560\n").unwrap();
        let install = Installation {
            install_dir: t.path().into(),
            data_dir: t.path().into(),
            version: "test".into(),
            source: "test".into(),
        };
        let mut journal = Journal {
            schema: 1,
            id: "test".into(),
            title: "test".into(),
            created: "test".into(),
            status: "applying".into(),
            installation: install,
            guest: None,
            entries: vec![Entry {
                label: "config".into(),
                target: Target::Config {
                    path: p.clone(),
                    edits: vec![Edit {
                        key: "ad".into(),
                        before: "1".into(),
                        after: "0".into(),
                    }],
                },
                before: save_value(
                    t.path(),
                    "before",
                    &Value::Bytes(b"ad=1\nport=5555\n".to_vec()),
                )
                .unwrap(),
                after: save_value(
                    t.path(),
                    "after",
                    &Value::Bytes(b"ad=0\nport=5555\n".to_vec()),
                )
                .unwrap(),
                status: "applying".into(),
                error: None,
            }],
        };
        restore_entries(
            t.path(),
            &mut journal,
            &Backend { guest: None },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"ad=1\nport=5560\n");
        restore_entries(
            t.path(),
            &mut journal,
            &Backend { guest: None },
            &mut |_| {},
        )
        .unwrap();
    }
    #[test]
    fn restore_refuses_newer_file() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("file");
        fs::write(&p, b"new").unwrap();
        let mut journal = Journal {
            schema: 1,
            id: "conflict".into(),
            title: "test".into(),
            created: "test".into(),
            status: "applied".into(),
            installation: Installation {
                install_dir: t.path().into(),
                data_dir: t.path().into(),
                source: "test".into(),
                version: "test".into(),
            },
            guest: None,
            entries: vec![Entry {
                label: "modified file".into(),
                target: Target::File { path: p.clone() },
                before: save_value(t.path(), "before", &Value::Bytes(b"original".to_vec()))
                    .unwrap(),
                after: save_value(t.path(), "after", &Value::Bytes(b"applied".to_vec())).unwrap(),
                status: "applied".into(),
                error: None,
            }],
        };
        assert!(
            restore_entries(
                t.path(),
                &mut journal,
                &Backend { guest: None },
                &mut |_| {}
            )
            .is_err()
        );
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(journal.status, "restore_incomplete");
        fs::write(&p, b"applied").unwrap();
        restore_entries(
            t.path(),
            &mut journal,
            &Backend { guest: None },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(p).unwrap(), b"original");
        assert_eq!(journal.status, "restored");
    }
    #[test]
    fn restore_attempts_every_entry_even_when_journal_storage_fails() {
        let t = tempfile::tempdir().unwrap();
        let mut entries = Vec::new();
        for i in 0..2 {
            let path = t.path().join(format!("target-{i}"));
            fs::write(&path, b"applied").unwrap();
            entries.push(Entry {
                label: format!("entry {i}"),
                target: Target::File { path },
                before: save_value(
                    t.path(),
                    &format!("{i}-before"),
                    &Value::Bytes(b"original".to_vec()),
                )
                .unwrap(),
                after: save_value(
                    t.path(),
                    &format!("{i}-after"),
                    &Value::Bytes(b"applied".to_vec()),
                )
                .unwrap(),
                status: "applied".into(),
                error: None,
            });
        }
        let mut journal = Journal {
            schema: 1,
            id: "storage-failure".into(),
            title: "test".into(),
            created: "test".into(),
            status: "applied".into(),
            installation: Installation {
                install_dir: t.path().into(),
                data_dir: t.path().into(),
                source: "test".into(),
                version: "test".into(),
            },
            guest: None,
            entries,
        };
        fs::create_dir(t.path().join("journal.json")).unwrap();
        assert!(
            restore_entries(
                t.path(),
                &mut journal,
                &Backend { guest: None },
                &mut |_| {}
            )
            .is_err()
        );
        for i in 0..2 {
            assert_eq!(
                fs::read(t.path().join(format!("target-{i}"))).unwrap(),
                b"original"
            );
        }
    }
}
