//! Fixed retention for completed recovery points. Live installation files are never deleted.
use crate::{platform, root_files, transaction};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub const KEEP_LATEST: usize = 3;
pub const WARNING_PREFIX: &str = "Backup cleanup needs attention:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Debloat,
    Root,
}

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub path: PathBuf,
    pub title: String,
    pub created: String,
    pub status: String,
    pub kind: Kind,
    pub can_delete: bool,
    pub problem: Option<String>,
    pub verified: bool,
}

fn journal_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Debloat => "journal.json",
        Kind::Root => "root-journal.json",
    }
}
fn folder(kind: Kind) -> &'static str {
    match kind {
        Kind::Debloat => "backups",
        Kind::Root => "root-backups",
    }
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("Missing recovery field: {key}"))
}
fn plain(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        let meta = fs::symlink_metadata(ancestor)?;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "Recovery path contains a link or junction: {}",
            ancestor.display()
        );
    }
    Ok(())
}
fn read(root: &Path, path: &Path, kind: Kind) -> Result<Value> {
    plain(path)?;
    let actual = platform::absolute(path)?;
    let base = platform::absolute(&root.join(folder(kind)))?;
    ensure!(
        actual
            .parent()
            .is_some_and(|p| platform::same_path(p, &base)),
        "Select a recovery folder from this app"
    );
    let name = actual
        .file_name()
        .context("Recovery folder has no name")?
        .to_string_lossy();
    ensure!(
        name.get(16..)
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            && name
                .get(..15)
                .is_some_and(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d-%H%M%S").is_ok()),
        "Unrecognized recovery folder name"
    );
    let journal_path = actual.join(journal_name(kind));
    plain(&journal_path)?;
    let journal = platform::read_journal(&journal_path)?;
    ensure!(
        journal.get("schema").and_then(Value::as_u64) == Some(1) && text(&journal, "id")? == name,
        "Recovery journal does not match its folder"
    );
    Ok(journal)
}
fn finished(status: &str) -> bool {
    matches!(status, "applied" | "restored" | "Complete" | "Restored")
}

pub fn list(root: &Path) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for kind in [Kind::Debloat, Kind::Root] {
        let base = root.join(folder(kind));
        if !base.try_exists()? {
            continue;
        }
        plain(&base)?;
        for entry in fs::read_dir(&base)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() && !entry.file_type()?.is_symlink() {
                continue;
            }
            let path = entry.path();
            let record = read(root, &path, kind)
                .and_then(|journal| {
                    let status = text(
                        &journal,
                        if kind == Kind::Root {
                            "stage"
                        } else {
                            "status"
                        },
                    )?
                    .to_owned();
                    let created = text(&journal, "created")?.to_owned();
                    chrono::DateTime::parse_from_rfc3339(&created)?;
                    let mut record = Record {
                        path: path.clone(),
                        title: text(
                            &journal,
                            if kind == Kind::Root {
                                "action"
                            } else {
                                "title"
                            },
                        )?
                        .to_owned(),
                        created,
                        can_delete: finished(&status) || status == "Deleting",
                        status,
                        kind,
                        problem: None,
                        verified: false,
                    };
                    record.problem = verify_record(root, &record, false)
                        .err()
                        .map(|e| format!("{e:#}"));
                    Ok(record)
                })
                .unwrap_or_else(|error| Record {
                    path,
                    kind,
                    title: "Damaged or incomplete recovery record".into(),
                    created: "Unknown".into(),
                    status: "Needs inspection".into(),
                    can_delete: false,
                    problem: Some(format!("{error:#}")),
                    verified: false,
                });
            records.push(record);
        }
    }
    records.sort_by(|a, b| {
        chrono::DateTime::parse_from_rfc3339(&b.created)
            .ok()
            .cmp(&chrono::DateTime::parse_from_rfc3339(&a.created).ok())
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok(records)
}

fn blob_path(dir: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.contains(['/', '\\', ':'])
            && name.ends_with(".bin")
            && Path::new(name).components().count() == 1,
        "Invalid recovery blob path"
    );
    Ok(dir.join(name))
}
fn disk_path(dir: &Path, saved: &str) -> Result<PathBuf> {
    let saved = PathBuf::from(saved);
    ensure!(
        saved.parent().is_some_and(|p| platform::same_path(p, dir)),
        "Disk recovery file escaped its backup folder"
    );
    let name = saved
        .file_name()
        .context("Missing disk recovery filename")?
        .to_string_lossy();
    ensure!(
        name.ends_with(".backup")
            && name
                .trim_end_matches(".backup")
                .bytes()
                .all(|b| b.is_ascii_digit()),
        "Unexpected disk recovery filename"
    );
    Ok(saved)
}
fn cloud_path(directory: &Value) -> Result<PathBuf> {
    let original = PathBuf::from(text(directory, "path")?);
    let stash = PathBuf::from(text(directory, "stash")?);
    let id = stash
        .parent()
        .and_then(Path::file_name)
        .context("Invalid cloud recovery path")?;
    uuid::Uuid::parse_str(&id.to_string_lossy()).context("Invalid cloud recovery ID")?;
    let expected = original
        .parent()
        .context("Cloud path has no parent")?
        .join(".BlueStacksDebloat-backups")
        .join(id)
        .join(original.file_name().context("Cloud path has no name")?);
    ensure!(
        original.is_absolute()
            && stash.is_absolute()
            && platform::same_path(&expected, &stash)
            && !stash
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Cloud recovery path escaped its owned archive"
    );
    Ok(stash)
}
fn verify_record(root: &Path, record: &Record, hashes: bool) -> Result<()> {
    let journal = read(root, &record.path, record.kind)?;
    ensure!(
        record.status != "Deleting",
        "Deletion was interrupted; the app will retry cleanup"
    );
    if record.kind == Kind::Root {
        let backups = journal
            .get("backups")
            .and_then(Value::as_array)
            .context("Missing disk backup list")?;
        ensure!(!backups.is_empty(), "No disk recovery files were recorded");
        for backup in backups {
            let path = disk_path(&record.path, text(backup, "saved")?)?;
            plain(&path).with_context(|| {
                format!("Missing or inaccessible recovery file {}", path.display())
            })?;
            ensure!(
                Some(path.metadata()?.len()) == backup.get("length").and_then(Value::as_u64),
                "Truncated disk recovery file: {}",
                path.display()
            );
            if hashes {
                ensure!(
                    root_files::hash_file(&path)? == text(backup, "sha256")?,
                    "Recovery file checksum mismatch: {}",
                    path.display()
                );
            }
        }
    } else {
        let entries = journal
            .get("entries")
            .and_then(Value::as_array)
            .context("Missing recovery entries")?;
        ensure!(!entries.is_empty(), "No recovery entries were recorded");
        for entry in entries {
            for key in ["before", "after"] {
                let saved = entry.get(key).context("Missing saved recovery value")?;
                if let Some(blob) = saved.get("Blob") {
                    let path = blob_path(&record.path, text(blob, "name")?)?;
                    plain(&path)
                        .with_context(|| format!("Missing recovery file {}", path.display()))?;
                    if hashes {
                        ensure!(
                            root_files::hash_file(&path)? == text(blob, "sha256")?,
                            "Recovery file checksum mismatch: {}",
                            path.display()
                        );
                    }
                } else {
                    ensure!(
                        saved.get("Inline").is_some(),
                        "Unrecognized saved recovery value"
                    );
                }
            }
            if let Some(directory) = entry.get("target").and_then(|v| v.get("Directory")) {
                let stash = cloud_path(directory)?;
                // Restored entries no longer need their moved cloud folder.
                let needs_stash = entry
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s == "applied");
                if needs_stash {
                    ensure!(
                        stash.is_dir(),
                        "Missing cloud recovery directory: {}",
                        stash.display()
                    );
                }
                if stash.try_exists()? {
                    plain(&stash)?;
                    if hashes {
                        ensure!(
                            transaction::tree_hash(&stash)? == text(directory, "fingerprint")?,
                            "Cloud recovery content changed: {}",
                            stash.display()
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn verify(root: &Path, path: &Path) -> Result<()> {
    let _lock = platform::lock(root)?;
    verify_locked(root, path)
}
pub fn audit(root: &Path) -> Result<Vec<Record>> {
    let _lock = platform::lock(root)?;
    let mut records = list(root)?;
    for record in &mut records {
        match verify_record(root, record, true) {
            Ok(()) => record.verified = true,
            Err(error) => record.problem = Some(format!("{error:#}")),
        }
    }
    Ok(records)
}
pub(crate) fn verify_locked(root: &Path, path: &Path) -> Result<()> {
    let record = list(root)?
        .into_iter()
        .find(|r| platform::same_path(&r.path, path))
        .context("Select an existing recovery point from this app")?;
    verify_record(root, &record, true)
}

struct DeletePlan {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    journal: Value,
}
fn walk(path: &Path, files: &mut Vec<PathBuf>, directories: &mut Vec<PathBuf>) -> Result<()> {
    plain(path)?;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        plain(&child)?;
        if entry.file_type()?.is_dir() {
            walk(&child, files, directories)?;
        } else {
            ensure!(entry.file_type()?.is_file(), "Unsupported recovery entry");
            files.push(child);
        }
    }
    directories.push(path.into());
    Ok(())
}
fn delete_plan(root: &Path, record: &Record) -> Result<DeletePlan> {
    let journal = read(root, &record.path, record.kind)?;
    let status = text(
        &journal,
        if record.kind == Kind::Root {
            "stage"
        } else {
            "status"
        },
    )?;
    ensure!(
        finished(status) || status == "Deleting",
        "This recovery operation is unfinished; resolve it before deletion"
    );
    let mut allowed = BTreeSet::from([journal_name(record.kind).to_owned()]);
    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut archive_roots = Vec::new();
    if record.kind == Kind::Root {
        for backup in journal
            .get("backups")
            .and_then(Value::as_array)
            .context("Missing disk backup list")?
        {
            let saved = disk_path(&record.path, text(backup, "saved")?)?;
            allowed.insert(
                saved
                    .file_name()
                    .context("Missing disk recovery name")?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    } else {
        for entry in journal
            .get("entries")
            .and_then(Value::as_array)
            .context("Missing recovery entries")?
        {
            for key in ["before", "after"] {
                if let Some(blob) = entry.get(key).and_then(|v| v.get("Blob")) {
                    let path = blob_path(&record.path, text(blob, "name")?)?;
                    allowed.insert(
                        path.file_name()
                            .context("Missing recovery name")?
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
            if let Some(directory) = entry.get("target").and_then(|v| v.get("Directory")) {
                let stash = cloud_path(directory)?;
                archive_roots.push(stash.clone());
                if status != "Deleting" && stash.try_exists()? {
                    plain(&stash)?;
                    ensure!(
                        status == "Deleting"
                            || transaction::tree_hash(&stash)? == text(directory, "fingerprint")?,
                        "Cloud recovery content changed; keeping it for inspection"
                    );
                    walk(&stash, &mut files, &mut directories)?;
                }
            }
        }
    }
    if status == "Deleting" {
        let cleanup = journal
            .get("_cleanup")
            .context("Interrupted cleanup has no deletion manifest; inspect it manually")?;
        files = serde_json::from_value(
            cleanup
                .get("files")
                .context("Missing cleanup file list")?
                .clone(),
        )?;
        directories = serde_json::from_value(
            cleanup
                .get("directories")
                .context("Missing cleanup directory list")?
                .clone(),
        )?;
        for path in files.iter().chain(&directories) {
            ensure!(
                path.is_absolute()
                    && !path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    && (path
                        .parent()
                        .is_some_and(|p| platform::same_path(p, &record.path))
                        || archive_roots.iter().any(|base| path.starts_with(base))),
                "Cleanup target escaped its recorded recovery paths"
            );
            ensure!(
                !platform::same_path(path, &record.path.join(journal_name(record.kind))),
                "Cleanup journal must be removed last"
            );
        }
    }
    for entry in fs::read_dir(&record.path)? {
        let entry = entry?;
        plain(&entry.path())?;
        ensure!(
            entry.file_type()?.is_file()
                && allowed.contains(&entry.file_name().to_string_lossy().into_owned()),
            "Unexpected content in recovery folder; keeping it for inspection"
        );
        if status != "Deleting" && entry.file_name() != journal_name(record.kind) {
            files.push(entry.path());
        }
    }
    Ok(DeletePlan {
        files,
        directories,
        journal,
    })
}
fn delete_locked(root: &Path, record: &Record) -> Result<()> {
    let mut plan = delete_plan(root, record)?;
    let key = if record.kind == Kind::Root {
        "stage"
    } else {
        "status"
    };
    plan.journal[key] = Value::String("Deleting".into());
    plan.journal["_cleanup"] =
        serde_json::json!({ "files": plan.files, "directories": plan.directories });
    let journal_path = record.path.join(journal_name(record.kind));
    platform::write_journal(&journal_path, &plan.journal)?;
    for file in plan.files {
        if file.try_exists()? {
            plain(&file)?;
            fs::remove_file(&file)
                .with_context(|| format!("Delete recovery file {}", file.display()))?;
        }
    }
    for directory in plan.directories {
        if directory.try_exists()? {
            plain(&directory)?;
            fs::remove_dir(&directory)?;
        }
    }
    // Keep the journal until everything it references has been removed.
    plain(&journal_path)?;
    fs::remove_file(journal_path)?;
    fs::remove_dir(&record.path)?;
    Ok(())
}
pub fn delete(root: &Path, path: &Path) -> Result<()> {
    let _lock = platform::lock(root)?;
    let selected = list(root)?
        .into_iter()
        .find(|r| platform::same_path(&r.path, path))
        .context("Select an existing recovery point from this app")?;
    delete_locked(root, &selected)
}
fn prune_locked(
    root: &Path,
    current: Option<&Path>,
    log: &mut impl FnMut(String),
) -> Result<usize> {
    let mut kept = 0;
    let mut deleted = 0;
    let mut problems = Vec::new();
    let mut records = list(root)?;
    // A clock correction must never retire the recovery point just created by this operation.
    if let Some(current) = current
        && let Some(index) = records
            .iter()
            .position(|record| platform::same_path(&record.path, current))
    {
        let record = records.remove(index);
        records.insert(0, record);
    }
    for record in records {
        let result = (|| -> Result<()> {
            if record.status == "Deleting" {
                delete_locked(root, &record)?;
                deleted += 1;
                return Ok(());
            }
            if !record.can_delete {
                anyhow::bail!(
                    "{}",
                    record
                        .problem
                        .as_deref()
                        .unwrap_or("Recovery is unfinished")
                );
            }
            if kept < KEEP_LATEST {
                log(format!("Checking retained backup: {}", record.title));
                verify_record(root, &record, true)?;
                kept += 1;
            } else {
                // A damaged old point cannot displace a newer verified recovery point.
                delete_locked(root, &record)?;
                deleted += 1;
                log(format!("Removed old backup: {}", record.title));
            }
            Ok(())
        })();
        if let Err(error) = result {
            problems.push(format!("{}: {error:#}", record.path.display()));
        }
    }
    ensure!(
        problems.is_empty(),
        "{WARNING_PREFIX} {}",
        problems.join("; ")
    );
    Ok(deleted)
}
pub fn prune(root: &Path, mut log: impl FnMut(String)) -> Result<usize> {
    let _lock = platform::lock(root)?;
    prune_locked(root, None, &mut log)
}
/// Caller holds the operation's exclusive lock. Cleanup failure does not undo verified changes.
pub(crate) fn finish_operation(
    root: &Path,
    _guard: &fs::File,
    current: &Path,
    log: &mut impl FnMut(String),
) {
    if let Err(error) = prune_locked(root, Some(current), log) {
        log(format!("{error:#}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point(root: &Path, index: u8, kind: Kind, status: &str, created: &str) -> PathBuf {
        let id = format!("20261007-1200{index:02}-{}", uuid::Uuid::new_v4());
        let path = root.join(folder(kind)).join(&id);
        fs::create_dir_all(&path).unwrap();
        let live = root.join(format!("live-{index}"));
        fs::write(&live, b"live data").unwrap();
        let mut journal = json!({"schema":1,"id":id,"created":created});
        if kind == Kind::Root {
            let saved = path.join("00.backup");
            fs::write(&saved, b"saved data").unwrap();
            journal["stage"] = status.into();
            journal["action"] = "Root fixture".into();
            journal["backups"] = json!([{"source":live,"saved":saved,"length":10,"sha256":platform::hash(b"saved data")}]);
        } else {
            fs::write(path.join("000-before.bin"), b"before").unwrap();
            fs::write(path.join("000-after.bin"), b"after").unwrap();
            journal["status"] = status.into();
            journal["title"] = "Debloat fixture".into();
            journal["installation"] =
                json!({"install_dir":root,"data_dir":root,"version":"fixture","source":"fixture"});
            journal["guest"] = Value::Null;
            journal["entries"] = json!([{"label":"fixture","target":{"File":{"path":live}},"status":status,"error":null,
                "before":{"Blob":{"name":"000-before.bin","sha256":platform::hash(b"before")}},
                "after":{"Blob":{"name":"000-after.bin","sha256":platform::hash(b"after")}}}]);
        }
        platform::write_journal(&path.join(journal_name(kind)), &journal).unwrap();
        path
    }

    #[test]
    fn retention_keeps_three_across_types_and_sorts_timezones() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let pending = point(root, 0, Kind::Debloat, "applying", "2026-10-07T09:00:00Z");
        let old = point(root, 1, Kind::Debloat, "restored", "2026-10-07T10:00:00Z");
        let latest_root = point(root, 2, Kind::Root, "Complete", "2026-10-07T10:30:00-04:00");
        let a = point(root, 3, Kind::Debloat, "applied", "2026-10-07T13:00:00Z");
        let b = point(root, 4, Kind::Debloat, "applied", "2026-10-07T14:00:00Z");
        assert!(prune(root, |_| {}).is_err()); // The unfinished point is reported, not ignored.
        assert!(!old.exists());
        for p in [pending, latest_root, a, b] {
            assert!(p.exists());
        }
        assert_eq!(
            list(root)
                .unwrap()
                .iter()
                .filter(|r| finished(&r.status))
                .count(),
            3
        );
        assert_eq!(fs::read(root.join("live-1")).unwrap(), b"live data");
    }

    #[test]
    fn damaged_new_backups_do_not_displace_good_recovery_points() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..3 {
            point(
                t.path(),
                i,
                Kind::Debloat,
                "applied",
                &format!("2026-10-07T10:00:0{i}Z"),
            );
        }
        let broken = point(t.path(), 3, Kind::Root, "Complete", "2026-10-07T11:00:00Z");
        fs::write(broken.join("00.backup"), b"short").unwrap();
        let corrupt = point(
            t.path(),
            4,
            Kind::Debloat,
            "applied",
            "2026-10-07T12:00:00Z",
        );
        fs::write(corrupt.join("000-before.bin"), b"damage").unwrap(); // Same length.
        let bad_json = point(
            t.path(),
            5,
            Kind::Debloat,
            "applied",
            "2026-10-07T13:00:00Z",
        );
        fs::write(bad_json.join("journal.json"), b"{partial").unwrap();
        assert!(prune(t.path(), |_| {}).is_err());
        let records = audit(t.path()).unwrap();
        assert_eq!(records.len(), 6);
        assert_eq!(records.iter().filter(|r| r.verified).count(), 3);
        assert_eq!(records.iter().filter(|r| r.problem.is_some()).count(), 3);
        assert!(delete(t.path(), &bad_json).is_err());
        delete(t.path(), &corrupt).unwrap(); // Owned damaged data can be deleted explicitly.
        assert_eq!(fs::read(t.path().join("live-4")).unwrap(), b"live data");
    }

    #[test]
    fn journal_checksum_detects_valid_json_corruption() {
        let t = tempfile::tempdir().unwrap();
        let path = point(
            t.path(),
            0,
            Kind::Debloat,
            "applied",
            "2026-10-07T10:00:00Z",
        );
        let file = path.join("journal.json");
        let bytes = fs::read_to_string(&file)
            .unwrap()
            .replace("Debloat fixture", "Altered fixture");
        fs::write(file, bytes).unwrap();
        assert!(verify(t.path(), &path).is_err());
        assert!(
            list(t.path()).unwrap()[0]
                .problem
                .as_ref()
                .unwrap()
                .contains("checksum mismatch")
        );
        assert!(delete(t.path(), &path).is_err());
    }

    #[test]
    fn interrupted_deletion_retries_and_preserves_unrecorded_files() {
        let t = tempfile::tempdir().unwrap();
        let path = point(
            t.path(),
            0,
            Kind::Debloat,
            "applied",
            "2026-10-07T10:00:00Z",
        );
        let record = list(t.path()).unwrap().remove(0);
        let mut plan = delete_plan(t.path(), &record).unwrap();
        plan.journal["status"] = "Deleting".into();
        plan.journal["_cleanup"] = json!({"files":plan.files,"directories":plan.directories});
        platform::write_journal(&path.join("journal.json"), &plan.journal).unwrap();
        fs::remove_file(&plan.files[0]).unwrap();
        fs::write(path.join("personal.txt"), b"preserve").unwrap();
        assert!(delete(t.path(), &path).is_err());
        assert_eq!(fs::read(path.join("personal.txt")).unwrap(), b"preserve");
        fs::remove_file(path.join("personal.txt")).unwrap();
        prune(t.path(), |_| {}).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn cloud_cleanup_deletes_only_the_verified_owned_archive() {
        let t = tempfile::tempdir().unwrap();
        let path = point(
            t.path(),
            0,
            Kind::Debloat,
            "applied",
            "2026-10-07T10:00:00Z",
        );
        let original = t.path().join("Programs/BlueStacks X");
        fs::create_dir_all(&original).unwrap();
        fs::write(original.join("live.txt"), b"live").unwrap();
        let stash = original
            .parent()
            .unwrap()
            .join(".BlueStacksDebloat-backups")
            .join(uuid::Uuid::new_v4().to_string())
            .join("BlueStacks X");
        fs::create_dir_all(&stash).unwrap();
        fs::write(stash.join("cloud.exe"), b"archived").unwrap();
        let mut journal = platform::read_journal(&path.join("journal.json")).unwrap();
        journal["entries"].as_array_mut().unwrap().push(json!({"status":"applied","target":{"Directory":{"path":original,"stash":stash,"fingerprint":transaction::tree_hash(&stash).unwrap()}},"before":{"Inline":{"Present":true}},"after":{"Inline":{"Present":false}}}));
        platform::write_journal(&path.join("journal.json"), &journal).unwrap();
        fs::write(stash.join("cloud.exe"), b"modified").unwrap();
        assert!(delete(t.path(), &path).is_err());
        fs::write(stash.join("cloud.exe"), b"archived").unwrap();
        delete(t.path(), &path).unwrap();
        assert!(!stash.exists());
        assert_eq!(fs::read(original.join("live.txt")).unwrap(), b"live");
    }

    #[test]
    fn restore_checks_every_blob_before_touching_live_files_and_cleanup_uses_lock() {
        let t = tempfile::tempdir().unwrap();
        let path = point(
            t.path(),
            0,
            Kind::Debloat,
            "applied",
            "2026-10-07T10:00:00Z",
        );
        fs::write(path.join("000-after.bin"), b"wrong").unwrap();
        let error = transaction::restore(&path, t.path(), |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("checksum mismatch"));
        assert_eq!(fs::read(t.path().join("live-0")).unwrap(), b"live data");
        let lock = platform::lock(t.path()).unwrap();
        assert!(delete(t.path(), &path).is_err());
        drop(lock);
        delete(t.path(), &path).unwrap();
    }

    #[test]
    fn malformed_paths_and_orphan_folders_remain_visible_and_cannot_escape() {
        let temp = tempfile::tempdir().unwrap();
        let point = point(
            temp.path(),
            0,
            Kind::Debloat,
            "applied",
            "2026-10-07T10:00:00Z",
        );
        let outside = temp.path().join("keep.bin");
        fs::write(&outside, b"preserve").unwrap();
        let mut journal = platform::read_journal(&point.join("journal.json")).unwrap();
        journal["entries"][0]["before"]["Blob"]["name"] = "../../keep.bin".into();
        platform::write_journal(&point.join("journal.json"), &journal).unwrap();
        let orphan = temp
            .path()
            .join("backups")
            .join(format!("20261007-120001-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&orphan).unwrap();
        fs::write(orphan.join("partial.bin"), b"incomplete").unwrap();
        let records = list(temp.path()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.problem.is_some()));
        assert!(delete(temp.path(), &point).is_err());
        assert!(delete(temp.path(), &orphan).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"preserve");
    }

    #[test]
    fn completed_rollback_also_enforces_retention() {
        use crate::transaction::{Operation, Plan, Target, Value as Stored};
        if !platform::is_admin() {
            return;
        } // Host writes require an administrator token.
        let temp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            point(
                temp.path(),
                i,
                Kind::Debloat,
                "applied",
                &format!("2020-01-01T00:00:0{i}Z"),
            );
        }
        let source = temp.path().join("source.txt");
        let blocked = temp.path().join("readonly.txt");
        fs::write(&source, b"original").unwrap();
        fs::write(&blocked, b"protected").unwrap();
        let original_permissions = blocked.metadata().unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&blocked, permissions).unwrap();
        let plan = Plan {
            installation: crate::discovery::Installation {
                install_dir: temp.path().join("install"),
                data_dir: temp.path().into(),
                version: "fixture".into(),
                source: "fixture".into(),
            },
            guest: None,
            title: "Rollback fixture".into(),
            notes: vec![],
            operations: vec![
                Operation {
                    label: "First write".into(),
                    target: Target::File {
                        path: source.clone(),
                    },
                    before: Stored::Bytes(b"original".to_vec()),
                    after: Stored::Bytes(b"changed".to_vec()),
                },
                Operation {
                    label: "Read-only destination".into(),
                    target: Target::File {
                        path: blocked.clone(),
                    },
                    before: Stored::Bytes(b"protected".to_vec()),
                    after: Stored::Bytes(b"changed".to_vec()),
                },
            ],
        };
        let result = transaction::apply(plan, temp.path(), |_| {});
        fs::set_permissions(&blocked, original_permissions).unwrap();
        assert!(format!("{:#}", result.unwrap_err()).contains("Rollback: complete"));
        assert_eq!(fs::read(source).unwrap(), b"original");
        assert_eq!(fs::read(blocked).unwrap(), b"protected");
        let records = list(temp.path()).unwrap();
        assert_eq!(records.len(), KEEP_LATEST);
        assert!(
            records
                .iter()
                .any(|r| r.title == "Rollback fixture" && r.status == "restored")
        );
    }

    #[test]
    fn current_operation_survives_a_backwards_clock_change() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            point(
                temp.path(),
                i,
                Kind::Debloat,
                "applied",
                &format!("2099-01-01T00:00:0{i}Z"),
            );
        }
        let live = temp.path().join("current.txt");
        fs::write(&live, b"before").unwrap();
        let plan = transaction::Plan {
            installation: crate::discovery::Installation {
                install_dir: temp.path().join("install"),
                data_dir: temp.path().into(),
                version: "fixture".into(),
                source: "fixture".into(),
            },
            guest: None,
            title: "Current operation".into(),
            notes: vec![],
            operations: vec![transaction::Operation {
                label: "Current write".into(),
                target: transaction::Target::File { path: live },
                before: transaction::Value::Bytes(b"before".to_vec()),
                after: transaction::Value::Bytes(b"after".to_vec()),
            }],
        };
        let backup = transaction::apply(plan, temp.path(), |_| {}).unwrap();
        assert!(backup.is_dir());
        verify(temp.path(), &backup).unwrap();
        assert_eq!(list(temp.path()).unwrap().len(), KEEP_LATEST);
    }
}
