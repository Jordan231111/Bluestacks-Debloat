use super::*;

fn report(journal: &Journal, backup: Option<PathBuf>, unchanged: usize) -> ApplyReport {
    ApplyReport {
        backup,
        applied: journal
            .entries
            .iter()
            .filter(|e| e.status == "applied")
            .map(|e| change_count(&e.target))
            .sum(),
        unchanged,
        failed: journal
            .issues
            .iter()
            .filter(|i| !i.skipped)
            .map(|i| i.changes)
            .sum(),
        skipped: journal
            .issues
            .iter()
            .filter(|i| i.skipped)
            .map(|i| i.changes)
            .sum(),
        needs_recovery: journal
            .entries
            .iter()
            .any(|e| matches!(e.status.as_str(), "applying" | "failed")),
        issues: journal.issues.clone(),
    }
}

fn issue(journal: &mut Journal, value: ApplyIssue, log: &mut impl FnMut(String)) {
    log(format!(
        "{}: {}: {}",
        if value.skipped { "Skipped" } else { "Failed" },
        value.label,
        value.error
    ));
    journal.issues.push(value);
}

fn checkpoint(dir: &Path, journal: &Journal) -> Result<()> {
    save(dir, journal).with_context(|| format!(
        "Recovery journal could not be saved; {} changes are already applied. No further changes were attempted. Recovery: {}",
        journal.entries.iter().filter(|e| e.status == "applied").map(|e| change_count(&e.target)).sum::<usize>(), dir.display()))
}

fn offline(target: &Target, installation: &Installation) -> bool {
    match target {
        Target::Config { .. } | Target::Directory { .. } | Target::RegistryTree { .. } => true,
        Target::File { path } => {
            platform::within_directory(path, &installation.install_dir)
                || platform::within_directory(path, &installation.data_dir)
        }
        _ => false,
    }
}

fn groups(plan: &Plan) -> Result<Vec<std::ops::Range<usize>>> {
    let mut explicit = plan.atomic_groups.clone();
    explicit.sort_by_key(|range| range.start);
    let mut end = 0;
    for range in &explicit {
        ensure!(
            range.start >= end && range.start < range.end && range.end <= plan.operations.len(),
            "Invalid or overlapping dependent operation group"
        );
        end = range.end;
    }
    let mut result = Vec::new();
    let mut at = 0;
    while at < plan.operations.len() {
        let range = explicit
            .iter()
            .find(|range| range.start == at)
            .cloned()
            .unwrap_or(at..at + 1);
        at = range.end;
        result.push(range);
    }
    Ok(result)
}

fn resource(target: &Target) -> String {
    let path = |path: &Path| {
        platform::absolute(path)
            .unwrap_or_else(|_| path.into())
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase()
    };
    match target {
        Target::File { path: file } | Target::Config { path: file, .. } => {
            format!("file:{}", path(file))
        }
        Target::Directory { path: dir, .. } => format!("directory:{}", path(dir)),
        _ => serde_json::to_string(target)
            .unwrap()
            .to_ascii_lowercase()
            .replace('/', "\\"),
    }
}

/// Prepare one independent target. Config edits that became stale are omitted
/// individually; valid edits still share one atomic file write and recovery copy.
fn prepare(
    mut operation: Operation,
    backend: &Backend,
    independent: bool,
    journal: &mut Journal,
    unchanged: &mut usize,
    log: &mut impl FnMut(String),
) -> Result<Option<Operation>> {
    if let Target::Directory {
        path, fingerprint, ..
    } = &mut operation.target
        && fingerprint.is_empty()
    {
        *fingerprint = tree_hash(path)?;
    }
    let current = backend.read(&operation.target)?;
    if let Target::Config { edits, .. } = &mut operation.target {
        let Value::Bytes(bytes) = &current else {
            bail!("Configuration disappeared")
        };
        let config = Config::parse(String::from_utf8(bytes.clone())?)?;
        let mut valid = Vec::new();
        for edit in edits.iter() {
            if config.get(&edit.key) == Some(edit.after.as_str()) {
                *unchanged += 1;
                continue;
            }
            match config.edit(std::slice::from_ref(edit), false) {
                Ok(_) => valid.push(edit.clone()),
                Err(error) if independent => issue(
                    journal,
                    ApplyIssue::failed(edit.key.clone(), format!("{error:#}"), 1),
                    log,
                ),
                Err(error) => return Err(error),
            }
        }
        *edits = valid;
        if edits.is_empty() {
            return Ok(None);
        }
        operation.label = format!("Update {} BlueStacks settings", edits.len());
        operation.after = Value::Bytes(config.edit(edits, false)?);
        operation.before = current;
    } else if current == operation.after {
        *unchanged += change_count(&operation.target);
        return Ok(None);
    } else {
        ensure!(
            current == operation.before,
            "{} changed since review; preserved without overwriting it",
            operation.label
        );
    }
    Ok(Some(operation))
}

pub fn apply(plan: Plan, root: &Path, mut log: impl FnMut(String)) -> Result<ApplyReport> {
    let ranges = groups(&plan)?;
    let mut resources = std::collections::BTreeSet::new();
    for operation in &plan.operations {
        ensure!(
            resources.insert(resource(&operation.target)),
            "Duplicate operation resource: {}",
            operation.label
        );
    }
    let _lock = platform::lock(root)?;
    let shutdown = if plan.guest.is_none() && !plan.operations.is_empty() {
        platform::require_admin()?;
        let paths = plan
            .operations
            .iter()
            .filter_map(|op| match &op.target {
                Target::Directory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        crate::shutdown::stop_with_cloud(&plan.installation, &paths, &mut log)
            .err()
            .map(|error| format!("{error:#}"))
    } else {
        None
    };
    let backend = Backend::new(plan.guest.as_ref())?;
    let id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    );
    let dir = root.join("backups").join(&id);
    let mut journal = Journal {
        schema: 1,
        id,
        title: plan.title,
        created: chrono::Utc::now().to_rfc3339(),
        status: "preparing".into(),
        installation: plan.installation,
        guest: plan.guest,
        entries: Vec::new(),
        issues: Vec::new(),
    };
    for value in plan.issues {
        issue(&mut journal, value, &mut log);
    }
    let mut operations = plan.operations.into_iter().map(Some).collect::<Vec<_>>();
    let mut prepared = Vec::new();
    let mut unchanged = 0;
    for range in ranges {
        let total = range
            .clone()
            .map(|index| change_count(&operations[index].as_ref().unwrap().target))
            .sum::<usize>();
        if let Some(error) = &shutdown
            && range.clone().any(|index| {
                offline(
                    &operations[index].as_ref().unwrap().target,
                    &journal.installation,
                )
            })
        {
            issue(
                &mut journal,
                ApplyIssue::skipped(
                    "Offline changes",
                    format!("Shutdown was not verified: {error}"),
                    total,
                ),
                &mut log,
            );
            continue;
        }
        let first = journal.entries.len();
        let mut failed_group = false;
        for index in range.clone() {
            let operation = operations[index].take().unwrap();
            let label = operation.label.clone();
            let count = change_count(&operation.target);
            if failed_group {
                issue(
                    &mut journal,
                    ApplyIssue::skipped(
                        label,
                        "A dependent operation could not be prepared",
                        count,
                    ),
                    &mut log,
                );
                continue;
            }
            let operation = match prepare(
                operation,
                &backend,
                range.len() == 1,
                &mut journal,
                &mut unchanged,
                &mut log,
            ) {
                Ok(Some(operation)) => operation,
                Ok(None) => continue,
                Err(error) => {
                    issue(
                        &mut journal,
                        ApplyIssue::failed(label, format!("{error:#}"), count),
                        &mut log,
                    );
                    failed_group = true;
                    continue;
                }
            };
            if journal.entries.is_empty() {
                fs::create_dir_all(&dir).with_context(|| {
                    format!(
                        "Cannot create recovery at {}; no live writes started",
                        dir.display()
                    )
                })?;
                checkpoint(&dir, &journal)?;
            }
            let before = save_value(&dir, &format!("{index:03}-before"), &operation.before)
                .with_context(|| {
                    format!(
                        "Could not prepare recovery; no live writes started. Inspect {}",
                        dir.display()
                    )
                })?;
            let after = save_value(&dir, &format!("{index:03}-after"), &operation.after)
                .with_context(|| {
                    format!(
                        "Could not prepare recovery; no live writes started. Inspect {}",
                        dir.display()
                    )
                })?;
            journal.entries.push(Entry {
                label: operation.label,
                target: operation.target,
                before,
                after,
                status: "prepared".into(),
                error: None,
            });
            checkpoint(&dir, &journal)?;
        }
        let entries = (first..journal.entries.len()).collect::<Vec<_>>();
        if failed_group {
            for &index in &entries {
                journal.entries[index].status = "not_applied".into();
                let entry = &journal.entries[index];
                let skipped = ApplyIssue::skipped(
                    entry.label.clone(),
                    "A dependent operation could not be prepared",
                    change_count(&entry.target),
                );
                issue(&mut journal, skipped, &mut log);
            }
        } else if !entries.is_empty() {
            prepared.push(entries);
        }
    }
    if journal.entries.is_empty() {
        let outcome = report(&journal, None, unchanged);
        log(outcome.summary());
        return Ok(outcome);
    }
    journal.status = "applying".into();
    checkpoint(&dir, &journal)?;
    for group in prepared {
        let mut applied_in_group: Vec<usize> = Vec::new();
        for (position, &index) in group.iter().enumerate() {
            let before = load_value(&dir, &journal.entries[index].before)?;
            let after = load_value(&dir, &journal.entries[index].after)?;
            let current = backend.read(&journal.entries[index].target);
            let mut attempted = false;
            let result = match current {
                Ok(current) if current == before => {
                    journal.entries[index].status = "applying".into();
                    checkpoint(&dir, &journal)?;
                    let entry = &journal.entries[index];
                    log(format!("Applying: {}", entry.label));
                    attempted = true;
                    backend
                        .write_checked(&entry.target, &before, &after)
                        .and_then(|_| {
                            ensure!(
                                backend.read(&entry.target)? == after,
                                "Read-back verification failed"
                            );
                            Ok(())
                        })
                }
                Ok(_) => Err(anyhow::anyhow!(
                    "Target changed during Apply; its newer value was preserved"
                )),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                if error.downcast_ref::<platform::WriteConflict>().is_some() {
                    attempted = false;
                }
                let details = format!("{error:#}");
                journal.entries[index].error = Some(details.clone());
                let entry = &journal.entries[index];
                let failure =
                    ApplyIssue::failed(entry.label.clone(), &details, change_count(&entry.target));
                issue(&mut journal, failure, &mut log);
                if attempted {
                    match restore_entry(&dir, &journal.entries[index], &backend, &mut log) {
                        Ok(()) => journal.entries[index].status = "not_applied".into(),
                        Err(restore) => {
                            journal.entries[index].status = "failed".into();
                            let details =
                                format!("{details}; recovery needs attention: {restore:#}");
                            journal.entries[index].error = Some(details.clone());
                            log(details);
                        }
                    }
                } else {
                    journal.entries[index].status = "not_applied".into();
                }
                for &prior in applied_in_group.iter().rev() {
                    match restore_entry(&dir, &journal.entries[prior], &backend, &mut log) {
                        Ok(()) => journal.entries[prior].status = "restored".into(),
                        Err(error) => {
                            journal.entries[prior].status = "failed".into();
                            journal.entries[prior].error = Some(format!("{error:#}"));
                        }
                    }
                    let entry = &journal.entries[prior];
                    let skipped = ApplyIssue::skipped(
                        entry.label.clone(),
                        "Reverted because a dependent operation failed",
                        change_count(&entry.target),
                    );
                    issue(&mut journal, skipped, &mut log);
                }
                for &later in &group[position + 1..] {
                    journal.entries[later].status = "not_applied".into();
                    let entry = &journal.entries[later];
                    let skipped = ApplyIssue::skipped(
                        entry.label.clone(),
                        "A dependent operation failed",
                        change_count(&entry.target),
                    );
                    issue(&mut journal, skipped, &mut log);
                }
                checkpoint(&dir, &journal)?;
                break;
            }
            journal.entries[index].status = "applied".into();
            applied_in_group.push(index);
            checkpoint(&dir, &journal)?;
        }
    }
    let outcome = report(&journal, Some(dir.clone()), unchanged);
    journal.status = if outcome.needs_recovery {
        "apply_incomplete"
    } else if outcome.applied == 0 {
        "restored"
    } else if outcome.has_issues() {
        "partial"
    } else {
        "applied"
    }
    .into();
    checkpoint(&dir, &journal)?;
    log(format!("{} Recovery: {}", outcome.summary(), dir.display()));
    if !outcome.needs_recovery {
        crate::backup_cleanup::finish_operation(root, &_lock, &dir, &mut log);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    fn plan(root: &Path, operations: Vec<Operation>) -> Plan {
        Plan {
            installation: Installation {
                install_dir: root.join("install"),
                data_dir: root.join("data"),
                version: "fixture".into(),
                source: "fixture".into(),
            },
            guest: None,
            title: "Partial-apply fixture".into(),
            operations,
            notes: vec![],
            issues: vec![],
            atomic_groups: vec![],
        }
    }
    fn file(root: &Path, name: &str) -> Operation {
        let path = root.join(name);
        fs::write(&path, b"before").unwrap();
        Operation {
            label: name.into(),
            target: Target::File { path },
            before: Value::Bytes(b"before".to_vec()),
            after: Value::Bytes(b"after".to_vec()),
        }
    }
    fn locked(root: &Path, name: &str) -> std::fs::File {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(root.join(name))
            .unwrap()
    }
    #[test]
    fn twenty_two_of_twenty_three_changes_survive_one_failure_and_restore_cleanly() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operations = (0..23)
            .map(|i| file(temp.path(), &format!("item-{i}")))
            .collect();
        let held = locked(temp.path(), "item-11");
        let mut logs = Vec::new();
        let outcome = apply(plan(temp.path(), operations), temp.path(), |line| {
            logs.push(line)
        })
        .unwrap();
        drop(held);
        assert_eq!(
            (outcome.applied, outcome.failed, outcome.skipped),
            (22, 1, 0)
        );
        assert!(!outcome.needs_recovery);
        assert!(logs.iter().any(|line| line.contains("Failed: item-11")));
        for i in 0..23 {
            assert_eq!(
                fs::read(temp.path().join(format!("item-{i}"))).unwrap(),
                if i == 11 {
                    b"before".as_slice()
                } else {
                    b"after".as_slice()
                }
            );
        }
        let backup = outcome.backup.unwrap();
        crate::backup_cleanup::verify(temp.path(), &backup).unwrap();
        restore(&backup, temp.path(), |_| {}).unwrap();
        for i in 0..23 {
            assert_eq!(
                fs::read(temp.path().join(format!("item-{i}"))).unwrap(),
                b"before"
            );
        }
    }
    #[test]
    fn dependent_group_rolls_back_itself_and_keeps_other_groups() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let mut plan = plan(
            temp.path(),
            [
                "independent-first",
                "cloud-file",
                "cloud-locked",
                "cloud-registry",
                "independent-last",
            ]
            .iter()
            .map(|name| file(temp.path(), name))
            .collect(),
        );
        plan.atomic_groups = std::iter::once(1..4).collect();
        let held = locked(temp.path(), "cloud-locked");
        let outcome = apply(plan, temp.path(), |_| {}).unwrap();
        drop(held);
        assert_eq!(
            (outcome.applied, outcome.failed, outcome.skipped),
            (2, 1, 2)
        );
        assert!(!outcome.needs_recovery);
        for name in ["cloud-file", "cloud-locked", "cloud-registry"] {
            assert_eq!(fs::read(temp.path().join(name)).unwrap(), b"before");
        }
        for name in ["independent-first", "independent-last"] {
            assert_eq!(fs::read(temp.path().join(name)).unwrap(), b"after");
        }
    }
    #[test]
    fn changed_target_is_skipped_without_overwriting_and_later_target_applies() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operations = vec![file(temp.path(), "stale"), file(temp.path(), "valid")];
        fs::write(temp.path().join("stale"), b"external edit").unwrap();
        let outcome = apply(plan(temp.path(), operations), temp.path(), |_| {}).unwrap();
        assert_eq!((outcome.applied, outcome.failed), (1, 1));
        assert_eq!(
            fs::read(temp.path().join("stale")).unwrap(),
            b"external edit"
        );
        assert_eq!(fs::read(temp.path().join("valid")).unwrap(), b"after");
    }
    #[test]
    fn stale_config_key_does_not_block_other_config_keys_or_overwrite_new_values() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config");
        fs::write(&path, b"a=1\nb=2\nport=5556\n").unwrap();
        let operation = Operation {
            label: "Config".into(),
            target: Target::Config {
                path: path.clone(),
                edits: vec![
                    Edit {
                        key: "a".into(),
                        before: "1".into(),
                        after: "0".into(),
                    },
                    Edit {
                        key: "b".into(),
                        before: "1".into(),
                        after: "0".into(),
                    },
                ],
            },
            before: Value::Bytes(b"a=1\nb=1\nport=5555\n".to_vec()),
            after: Value::Bytes(vec![]),
        };
        let outcome = apply(plan(temp.path(), vec![operation]), temp.path(), |_| {}).unwrap();
        assert_eq!((outcome.applied, outcome.failed), (1, 1));
        assert_eq!(fs::read(&path).unwrap(), b"a=0\nb=2\nport=5556\n");
        restore(&outcome.backup.unwrap(), temp.path(), |_| {}).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"a=1\nb=2\nport=5556\n");
    }
    #[test]
    fn unavailable_planning_step_is_reported_while_available_changes_apply() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let mut plan = plan(temp.path(), vec![file(temp.path(), "valid")]);
        plan.issues.push(ApplyIssue::skipped(
            "Future player patch",
            "Unsupported signature",
            1,
        ));
        let outcome = apply(plan, temp.path(), |_| {}).unwrap();
        assert_eq!(
            (outcome.applied, outcome.failed, outcome.skipped),
            (1, 0, 1)
        );
        let journal: Journal = serde_json::from_value(
            platform::read_journal(&outcome.backup.unwrap().join("journal.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(journal.status, "partial");
        assert_eq!(journal.issues[0].label, "Future player patch");
    }
    #[test]
    fn no_write_when_recovery_storage_is_unavailable() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operations = vec![file(temp.path(), "valid")];
        fs::write(temp.path().join("backups"), b"occupied").unwrap();
        assert!(apply(plan(temp.path(), operations), temp.path(), |_| {}).is_err());
        assert_eq!(fs::read(temp.path().join("valid")).unwrap(), b"before");
    }
    #[test]
    fn last_moment_external_edit_is_preserved_and_does_not_block_other_targets() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operations = vec![
            file(temp.path(), "changed-during-apply"),
            file(temp.path(), "valid"),
        ];
        let outcome = apply(plan(temp.path(), operations), temp.path(), |line| {
            if line == "Applying: changed-during-apply" {
                fs::write(temp.path().join("changed-during-apply"), b"external value").unwrap();
            }
        })
        .unwrap();
        assert_eq!((outcome.applied, outcome.failed), (1, 1));
        assert!(!outcome.needs_recovery);
        assert_eq!(
            fs::read(temp.path().join("changed-during-apply")).unwrap(),
            b"external value"
        );
        assert_eq!(fs::read(temp.path().join("valid")).unwrap(), b"after");
    }
    #[test]
    fn creating_an_absent_file_has_an_exact_reversible_backup() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hosts");
        let operation = Operation {
            label: "Create hosts".into(),
            target: Target::File { path: path.clone() },
            before: Value::Missing,
            after: Value::Bytes(crate::hosts::update(b"", crate::hosts::DOMAINS).unwrap()),
        };
        let outcome = apply(plan(temp.path(), vec![operation]), temp.path(), |_| {}).unwrap();
        assert_eq!(outcome.applied, 1);
        assert!(path.is_file());
        restore(&outcome.backup.unwrap(), temp.path(), |_| {}).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn already_completed_changes_do_not_create_redundant_backups() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operation = file(temp.path(), "already-set");
        fs::write(temp.path().join("already-set"), b"after").unwrap();
        let outcome = apply(plan(temp.path(), vec![operation]), temp.path(), |_| {}).unwrap();
        assert_eq!((outcome.applied, outcome.unchanged), (0, 1));
        assert!(outcome.backup.is_none());
        assert!(!temp.path().join("backups").exists());
    }
    #[test]
    fn interrupted_journal_write_stops_new_writes_and_retains_recoverable_progress() {
        if !platform::is_admin() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let operations = vec![file(temp.path(), "first"), file(temp.path(), "later")];
        let mut held = None;
        let mut backup = None;
        let result = apply(plan(temp.path(), operations), temp.path(), |line| {
            if line.starts_with("Applying:") && held.is_none() {
                let path = fs::read_dir(temp.path().join("backups"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                held = Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .share_mode(3)
                        .open(path.join("journal.json"))
                        .unwrap(),
                );
                backup = Some(path);
            }
        });
        drop(held);
        assert!(format!("{:#}", result.unwrap_err()).contains("already applied"));
        assert_eq!(fs::read(temp.path().join("first")).unwrap(), b"after");
        assert_eq!(fs::read(temp.path().join("later")).unwrap(), b"before");
        restore(&backup.unwrap(), temp.path(), |_| {}).unwrap();
        assert_eq!(fs::read(temp.path().join("first")).unwrap(), b"before");
    }
    #[test]
    fn invalid_dependency_ranges_and_duplicate_resources_fail_before_changes() {
        let temp = tempfile::tempdir().unwrap();
        let operation = file(temp.path(), "same");
        assert!(
            apply(
                plan(temp.path(), vec![operation.clone(), operation.clone()]),
                temp.path(),
                |_| {}
            )
            .is_err()
        );
        let mut plan = plan(temp.path(), vec![operation]);
        plan.atomic_groups = std::iter::once(0..2).collect();
        assert!(apply(plan, temp.path(), |_| {}).is_err());
        assert_eq!(fs::read(temp.path().join("same")).unwrap(), b"before");
    }
}
