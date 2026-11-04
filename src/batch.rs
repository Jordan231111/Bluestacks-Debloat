//! Instance queues keep guest identities and recovery separate, with one active
//! temporary player at a time. Players already open are left running.
use crate::{
    adb::{Client, Guest, Package},
    discovery::{self, Installation},
    engine, network, platform,
    transaction::{self, ApplyReport, Plan},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::Path,
    process::Child,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct Control(Arc<AtomicBool>);
impl Control {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceScan {
    pub instance: String,
    pub guest: Option<Guest>,
    pub packages: Vec<Package>,
    pub error: Option<String>,
    /// Exact read-only snapshot. Review filters this without booting Android again.
    #[serde(skip)]
    pub snapshot: Option<Plan>,
}

#[derive(Clone, Debug)]
pub struct InstancePlan {
    pub instance: String,
    pub plan: Option<Plan>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BatchPlan {
    pub entries: Vec<InstancePlan>,
    pub start_offline: bool,
}
impl BatchPlan {
    pub fn change_count(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|e| e.plan.as_ref())
            .flat_map(|p| &p.operations)
            .map(|o| transaction::change_count(&o.target))
            .sum()
    }
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({"start_offline": self.start_offline, "changes": self.change_count(),
            "instances": self.entries.iter().map(|e| serde_json::json!({"instance": e.instance,
                "plan": e.plan.as_ref().map(Plan::summary), "error": e.error})).collect::<Vec<_>>()})
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceResult {
    pub instance: String,
    pub report: Option<ApplyReport>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BatchReport {
    pub instances: Vec<InstanceResult>,
}
impl BatchReport {
    pub fn has_issues(&self) -> bool {
        self.instances
            .iter()
            .any(|e| e.error.is_some() || e.report.as_ref().is_some_and(ApplyReport::has_issues))
    }
    pub fn summary(&self) -> String {
        let ready = self
            .instances
            .iter()
            .filter(|e| e.error.is_none() && e.report.as_ref().is_some_and(|r| !r.has_issues()))
            .count();
        format!(
            "{ready}/{} {} verified{}",
            self.instances.len(),
            if self.instances.len() == 1 {
                "instance"
            } else {
                "instances"
            },
            if self.has_issues() {
                "; see per-instance results"
            } else {
                "."
            }
        )
    }
}

struct StartedPlayer(Child);
impl StartedPlayer {
    fn stop(&mut self) -> Result<()> {
        if self.0.try_wait()?.is_none() {
            // The process handle belongs exclusively to this queue's launch.
            // Never enumerate/kill other players or stop shared services here.
            self.0
                .kill()
                .context("Close the player started by this queue")?;
        }
        self.0.wait()?;
        Ok(())
    }
}
impl Drop for StartedPlayer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn with_running<T, L: FnMut(String)>(
    install: &Installation,
    instance: &str,
    start_offline: bool,
    log: &mut L,
    action: impl FnOnce(&Client, &mut L) -> Result<T>,
) -> Result<T> {
    let running = discovery::processes(install)?;
    let is_running = running.iter().any(|p| {
        p.name.eq_ignore_ascii_case("HD-Player.exe") && p.instance.as_deref() == Some(instance)
    });
    let mut started = if is_running {
        None
    } else {
        ensure!(
            start_offline,
            "{instance} is stopped. Enable 'Start stopped instances' or start it in BlueStacks."
        );
        ensure!(
            !running
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case("HD-Player.exe") && p.instance.is_none()),
            "A running player cannot be identified. Refresh or close that player before starting another instance."
        );
        require_start_port_free(install, instance)?;
        log(format!("{instance}: starting; waiting for Android…"));
        Some(StartedPlayer(discovery::launch_owned(install, instance)?))
    };
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(120);
        let client = loop {
            match Client::connect(install, instance) {
                Ok(client) => break client,
                Err(error) if started.is_some() && Instant::now() < deadline => {
                    ensure!(
                        started.as_mut().unwrap().0.try_wait()?.is_none(),
                        "{instance}: player exited before Android became ready: {error:#}"
                    );
                    std::thread::sleep(Duration::from_secs(1));
                }
                Err(error) => return Err(error),
            }
        };
        let result = action(&client, log);
        // A framework shutdown flushes delayed settings/package writes as well
        // as filesystem buffers. Only a player started by this queue is closed.
        if started.is_some() {
            log(format!("{instance}: saving Android state before closing…"));
            client
                .prepare_close()
                .context("Save Android state before closing the queued player")?;
        }
        result
    })();
    if let Some(player) = &mut started {
        player.stop()?;
        log(format!(
            "{instance}: closed the player started by this queue"
        ));
    }
    result
}

fn require_start_port_free(install: &Installation, instance: &str) -> Result<()> {
    let config = crate::config::Config::read(&install.conf())?;
    if let Some(port) = config
        .get(&format!("bst.instance.{instance}.adb_port"))
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
    {
        ensure!(
            !network::connections()?.iter().any(|c| c.state == 2
                && c.local_port == port
                && matches!(c.local_ip.as_str(), "127.0.0.1" | "0.0.0.0")),
            "Cannot start {instance}: its configured ADB port {port} is already in use. Stop the instance or application using this port, or clone this instance in Multi-instance Manager for a separate port, then scan again."
        );
    }
    Ok(())
}

fn validate_names(install: &Installation, names: &[String]) -> Result<Vec<String>> {
    ensure!(!names.is_empty(), "Select at least one instance");
    let conf = crate::config::Config::read(&install.conf())?;
    ensure!(
        conf.get("bst.enable_adb_access") == Some("1"),
        "Android tools (ADB) are disabled. Select 'Enable local Android tools (ADB)' in Debloat & tune, review and apply, then scan Android apps."
    );
    let known = discovery::instances(install, &conf);
    let names = names.iter().cloned().collect::<BTreeSet<_>>();
    for name in &names {
        ensure!(
            known.iter().any(|i| &i.name == name),
            "Unknown instance: {name}"
        );
    }
    Ok(names.into_iter().collect())
}

pub fn scan(
    install: &Installation,
    names: &[String],
    start_offline: bool,
    control: &Control,
    mut log: impl FnMut(String),
) -> Result<Vec<InstanceScan>> {
    let names = validate_names(install, names)?;
    let _guard = platform::lock(&platform::state_dir())?;
    Ok(names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if control.cancelled() {
                return InstanceScan {
                    instance: name.clone(),
                    guest: None,
                    packages: vec![],
                    error: Some("Not scanned: queue stopped after the previous instance.".into()),
                    snapshot: None,
                };
            }
            log(format!("Scanning {}/{}: {name}", i + 1, names.len()));
            match with_running(install, name, start_offline, &mut log, |client, _| {
                let packages = client.candidates()?;
                let all = packages.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
                let plan = engine::guest_plan_connected(client, &all, true)?;
                Ok((client.guest.clone(), packages, plan))
            }) {
                Ok((guest, packages, plan)) => InstanceScan {
                    instance: name.clone(),
                    guest: Some(guest),
                    packages,
                    error: None,
                    snapshot: Some(plan),
                },
                Err(error) => InstanceScan {
                    instance: name.clone(),
                    guest: None,
                    packages: vec![],
                    error: Some(format!("{error:#}")),
                    snapshot: None,
                },
            }
        })
        .collect())
}

pub fn review(
    scans: &[InstanceScan],
    selected: &[String],
    animations: bool,
    start_offline: bool,
    control: &Control,
    mut log: impl FnMut(String),
) -> Result<BatchPlan> {
    ensure!(!scans.is_empty(), "Scan the selected instances first");
    let entries = scans
        .iter()
        .enumerate()
        .map(|(i, scan)| {
            log(format!(
                "Reviewing {}/{}: {}",
                i + 1,
                scans.len(),
                scan.instance
            ));
            let result = (|| {
                ensure!(
                    !control.cancelled(),
                    "Not reviewed: queue stopped after the previous instance."
                );
                let guest = scan.guest.as_ref().context(
                    scan.error
                        .clone()
                        .unwrap_or_else(|| "Scan this instance first".into()),
                )?;
                let mut plan = scan
                    .snapshot
                    .clone()
                    .context("Scan this instance again to refresh its Android snapshot")?;
                ensure!(
                    guest.instance == scan.instance
                        && plan.guest.as_ref().is_some_and(
                            |g| g.instance == scan.instance && g.identity == guest.identity
                        ),
                    "Android snapshot belongs to another instance"
                );
                plan.operations.retain(|operation| match &operation.target {
                    transaction::Target::Package { name } => selected.contains(name),
                    transaction::Target::Setting { .. } => animations,
                    _ => false,
                });
                if !animations {
                    plan.issues
                        .retain(|issue| !issue.label.starts_with("Android animation:"));
                }
                for name in selected.iter().collect::<BTreeSet<_>>() {
                    if !crate::adb::reviewed_package(name) {
                        plan.issues.push(transaction::ApplyIssue::skipped(
                            format!("Disable {name}"),
                            "Package is outside the reviewed candidate list; left unchanged",
                            1,
                        ));
                    }
                }
                Ok(plan)
            })();
            match result {
                Ok(plan) => InstancePlan {
                    instance: scan.instance.clone(),
                    plan: Some(plan),
                    error: None,
                },
                Err(error) => InstancePlan {
                    instance: scan.instance.clone(),
                    plan: None,
                    error: Some(format!("{error:#}")),
                },
            }
        })
        .collect();
    Ok(BatchPlan {
        entries,
        start_offline,
    })
}

pub fn apply(
    batch: BatchPlan,
    root: &Path,
    control: &Control,
    mut log: impl FnMut(String),
) -> Result<BatchReport> {
    let mut names = BTreeSet::new();
    for entry in &batch.entries {
        ensure!(
            entry.plan.is_some() || entry.error.is_some(),
            "Android queue entry has neither a review nor an error"
        );
        ensure!(
            names.insert(&entry.instance),
            "Duplicate instance in Android queue"
        );
        if let Some(plan) = &entry.plan {
            ensure!(
                plan.guest
                    .as_ref()
                    .is_some_and(|guest| guest.instance == entry.instance),
                "Android queue contains a plan for another instance"
            );
        }
    }
    let guard = platform::lock(root)?;
    let id = uuid::Uuid::new_v4().to_string();
    let total = batch.entries.len();
    let mut results = Vec::new();
    for (i, entry) in batch.entries.into_iter().enumerate() {
        log(format!("Applying {}/{total}: {}", i + 1, entry.instance));
        let mut result = InstanceResult {
            instance: entry.instance.clone(),
            report: None,
            error: entry.error,
        };
        if control.cancelled() {
            result.error = Some(
                "Skipped: queue stopped after the previous instance. No changes attempted here."
                    .into(),
            );
            results.push(result);
            continue;
        }
        if let Some(plan) = entry.plan {
            if plan.operations.is_empty() {
                result.report = Some(ApplyReport {
                    backup: None,
                    applied: 0,
                    unchanged: 0,
                    failed: plan
                        .issues
                        .iter()
                        .filter(|i| !i.skipped)
                        .map(|i| i.changes)
                        .sum(),
                    skipped: plan
                        .issues
                        .iter()
                        .filter(|i| i.skipped)
                        .map(|i| i.changes)
                        .sum(),
                    needs_recovery: false,
                    issues: plan.issues,
                });
                results.push(result);
                continue;
            }
            // Refuse a malformed batch before a host plan could stop every player.
            let guest = plan
                .guest
                .clone()
                .context("A queued Android plan has no guest identity")?;
            let install = guest.installation.clone();
            let applied = with_running(
                &install,
                &entry.instance,
                batch.start_offline,
                &mut log,
                |client, log| {
                    ensure!(
                        guest.instance == entry.instance && client.guest.identity == guest.identity,
                        "Android identity changed since review; scan and review again"
                    );
                    let report =
                        match transaction::apply_locked(plan, root, &guard, Some(&id), &mut *log) {
                            Ok(report) => report,
                            Err(error) => {
                                // Structural/recovery failures must stop new transactions.
                                // Ordinary package failures are returned in ApplyReport.
                                control.cancel();
                                return Err(error);
                            }
                        };
                    if report.needs_recovery {
                        control.cancel();
                        log("Queue stopped because recovery needs attention. Completed changes remain recorded.".into());
                    }
                    result.report = Some(report);
                    Ok(())
                },
            );
            if let Err(error) = applied {
                if result.report.is_some() {
                    control.cancel();
                }
                result.error = Some(format!("{error:#}"));
            }
        }
        results.push(result);
    }
    Ok(BatchReport { instances: results })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::{Operation, Target, Value};

    #[test]
    fn occupied_adb_port_is_reported_before_a_player_is_launched() {
        let root = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            root.path().join("bluestacks.conf"),
            format!("bst.instance.Nougat64.adb_port=\"{port}\"\n"),
        )
        .unwrap();
        let install = Installation {
            install_dir: root.path().into(),
            data_dir: root.path().into(),
            version: "fixture".into(),
            source: "fixture".into(),
        };
        let error = require_start_port_free(&install, "Nougat64").unwrap_err();
        assert!(error.to_string().contains(&port.to_string()));
        drop(listener);
        require_start_port_free(&install, "Nougat64").unwrap();
    }

    fn scanned(name: &str) -> InstanceScan {
        let guest = Guest {
            installation: Installation {
                install_dir: "missing-installation".into(),
                data_dir: "missing-data".into(),
                version: "fixture".into(),
                source: "fixture".into(),
            },
            instance: name.into(),
            identity: format!("identity-{name}"),
        };
        InstanceScan {
            instance: name.into(),
            guest: Some(guest.clone()),
            error: None,
            packages: vec![Package {
                name: "com.android.egg".into(),
                enabled: 0,
                reason: "Optional app".into(),
                recommended: false,
            }],
            snapshot: Some(Plan {
                installation: guest.installation.clone(),
                guest: Some(guest),
                title: name.into(),
                notes: vec![],
                issues: vec![],
                atomic_groups: vec![],
                operations: vec![
                    Operation {
                        label: "Disable egg".into(),
                        target: Target::Package {
                            name: "com.android.egg".into(),
                        },
                        before: Value::Number(0),
                        after: Value::Number(3),
                    },
                    Operation {
                        label: "Animation".into(),
                        target: Target::Setting {
                            name: "window_animation_scale".into(),
                        },
                        before: Value::Text(None),
                        after: Value::Text(Some("0.5".into())),
                    },
                ],
            }),
        }
    }

    #[test]
    fn review_uses_exact_scan_values_offline_and_keeps_unavailable_instances_visible() {
        let scan = scanned("Pie64");
        let failed = InstanceScan {
            instance: "Rvc64".into(),
            guest: None,
            packages: vec![],
            error: Some("Offline".into()),
            snapshot: None,
        };
        let review = review(
            &[scan, failed],
            &["com.android.egg".into(), "com.android.traceur".into()],
            false,
            true,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(review.change_count(), 1);
        let plan = review.entries[0].plan.as_ref().unwrap();
        assert_eq!(plan.operations[0].before, Value::Number(0));
        assert_eq!(plan.guest.as_ref().unwrap().identity, "identity-Pie64");
        assert_eq!(review.entries[1].error.as_deref(), Some("Offline"));
        assert!(
            plan.issues.is_empty(),
            "An app absent on one Android version is expected"
        );
    }

    #[test]
    fn explicit_unreviewed_packages_are_reported_without_becoming_mutations() {
        let plan = review(
            &[scanned("Pie64")],
            &["com.example.game".into()],
            false,
            true,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(plan.change_count(), 0);
        let issues = &plan.entries[0].plan.as_ref().unwrap().issues;
        assert_eq!(issues.len(), 1);
        assert!(issues[0].skipped && issues[0].label.contains("com.example.game"));
    }

    #[test]
    fn already_clean_instances_are_not_booted_during_a_mixed_queue() {
        let root = tempfile::tempdir().unwrap();
        let plan = review(
            &[scanned("Pie64")],
            &[],
            false,
            true,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        // The fixture installation does not exist: connecting or launching would fail.
        let result = apply(plan, root.path(), &Control::default(), |_| {}).unwrap();
        assert!(!result.has_issues());
        assert_eq!(result.instances.len(), 1);
        assert!(
            result.instances[0]
                .report
                .as_ref()
                .unwrap()
                .backup
                .is_none()
        );
        assert!(!root.path().join("backups").exists());
    }

    #[test]
    fn cancelled_apply_never_starts_android_or_creates_a_recovery_point() {
        let t = tempfile::tempdir().unwrap();
        let sentinel = t.path().join("existing-data");
        std::fs::write(&sentinel, b"preserve").unwrap();
        let batch = review(
            &[scanned("Pie64"), scanned("Rvc64")],
            &[],
            true,
            true,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        let control = Control::default();
        control.cancel();
        let result = apply(batch, t.path(), &control, |_| {}).unwrap();
        assert!(result.has_issues());
        assert_eq!(result.instances.len(), 2);
        assert!(
            result
                .instances
                .iter()
                .all(|r| r.report.is_none() && r.error.as_ref().unwrap().contains("Skipped"))
        );
        assert!(!t.path().join("backups").exists());
        assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve");
    }

    #[test]
    fn mismatched_and_duplicate_instances_fail_before_starting_or_writing() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("state");
        let plan = review(
            &[scanned("Pie64")],
            &[],
            true,
            true,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        let mut wrong = plan.clone();
        wrong.entries[0].instance = "Rvc64".into();
        assert!(apply(wrong, &root, &Control::default(), |_| {}).is_err());
        let mut duplicate = plan;
        duplicate.entries.push(duplicate.entries[0].clone());
        assert!(apply(duplicate, &root, &Control::default(), |_| {}).is_err());
        assert!(!root.exists());
    }
}
