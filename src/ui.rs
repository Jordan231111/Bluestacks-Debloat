use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
mod action_bar;
mod android_panel;
mod backups_panel;
mod host_panel;
mod responsive;
mod root_panel;
#[cfg(test)]
mod tests;
use bluestacks_debloat::{
    adb::{Client, Package},
    backup_cleanup, batch,
    discovery::{self, Installation, Snapshot},
    engine::{self, HostOptions, Performance},
    network, platform, rooting,
    transaction::{self, Plan, Target},
};
use eframe::egui::{self, Color32, RichText};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

const ACCENT: Color32 = Color32::from_rgb(116, 208, 194);
const MUTED: Color32 = Color32::from_rgb(156, 169, 190);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Tab {
    Overview,
    Host,
    Android,
    Cloud,
    Network,
    Root,
    Backups,
}
#[derive(Serialize, Deserialize)]
struct Preferences {
    instance: String,
    install_path: String,
    conf_path: String,
    text_scale: f32,
    tab: Tab,
    #[serde(default)]
    instances: Option<BTreeSet<String>>,
}
enum Reply {
    Scanned(Snapshot),
    Preview(Plan),
    Applied(transaction::ApplyReport),
    Packages(Vec<batch::InstanceScan>),
    BatchPreview(batch::BatchPlan),
    BatchApplied(batch::BatchReport),
    Network(network::Report),
    Backups(Vec<backup_cleanup::Record>),
    Done(String),
    RootInfo(rooting::RootInfo),
    RootStatus(rooting::Verification),
    RootBackups(Vec<rooting::RootBackupInfo>),
}
enum Event {
    Log(String),
    Refreshed(Snapshot),
    Finished(Box<std::result::Result<Reply, String>>),
}
struct App {
    tab: Tab,
    snapshot: Option<Snapshot>,
    instance: String,
    instances: BTreeSet<String>,
    selection_initialized: bool,
    instance_filter: String,
    install_path: String,
    conf_path: String,
    options: HostOptions,
    plan: Option<Plan>,
    applied: Option<action_bar::AppliedChanges>,
    scroll_to_preview: bool,
    packages: Vec<Package>,
    scans: Vec<batch::InstanceScan>,
    batch_plan: Option<batch::BatchPlan>,
    batch_report: Option<batch::BatchReport>,
    start_offline: bool,
    queue_control: Option<batch::Control>,
    selected: BTreeSet<String>,
    animations: bool,
    report: Option<network::Report>,
    backups: Vec<backup_cleanup::Record>,
    delete_backup: Option<PathBuf>,
    backup_warning: Option<String>,
    logs: Vec<String>,
    error: Option<String>,
    status: String,
    busy: bool,
    root_hosts: bool,
    root_isolate: bool,
    maximum_selected: bool,
    root_info: Option<rooting::RootInfo>,
    root_status: Option<rooting::Verification>,
    root_backups: Vec<rooting::RootBackupInfo>,
    root_repair: bool,
    layout_initialized: bool,
    text_scale: f32,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    root: PathBuf,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(20, 25, 34);
        style.visuals.window_fill = Color32::from_rgb(25, 31, 42);
        style.visuals.selection.bg_fill = Color32::from_rgb(40, 96, 94);
        style.visuals.hyperlink_color = ACCENT;
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 9.0);
        style.wrap_mode = Some(egui::TextWrapMode::Wrap);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(27.0));
        cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
        let mut app = Self::blank(platform::state_dir());
        if let Ok(bytes) = std::fs::read(app.root.join("ui-preferences.json"))
            && let Ok(saved) = serde_json::from_slice::<Preferences>(&bytes)
        {
            app.instance = saved.instance;
            if let Some(instances) = saved.instances {
                app.instances = instances;
                app.selection_initialized = true;
            }
            app.install_path = saved.install_path;
            app.conf_path = saved.conf_path;
            app.text_scale = saved.text_scale.clamp(1.0, 2.0);
            app.tab = saved.tab;
            cc.egui_ctx.set_zoom_factor(app.text_scale);
        }
        app.scan(&cc.egui_ctx);
        app
    }
    fn blank(root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tab: Tab::Overview,
            snapshot: None,
            instance: String::new(),
            instances: BTreeSet::new(),
            selection_initialized: false,
            instance_filter: String::new(),
            install_path: String::new(),
            conf_path: String::new(),
            options: HostOptions::default(),
            plan: None,
            applied: None,
            scroll_to_preview: false,
            packages: vec![],
            scans: vec![],
            batch_plan: None,
            batch_report: None,
            start_offline: true,
            queue_control: None,
            selected: BTreeSet::new(),
            animations: true,
            report: None,
            backups: vec![],
            delete_backup: None,
            backup_warning: None,
            logs: vec![],
            error: None,
            status: "Finding BlueStacks…".into(),
            busy: false,
            root_hosts: false,
            root_isolate: false,
            maximum_selected: false,
            root_info: None,
            root_status: None,
            root_backups: Vec::new(),
            root_repair: false,
            layout_initialized: false,
            text_scale: 1.0,
            tx,
            rx,
            root,
        }
    }
    fn save_preferences(&self) {
        let prefs = Preferences {
            instance: self.instance.clone(),
            install_path: self.install_path.clone(),
            conf_path: self.conf_path.clone(),
            text_scale: self.text_scale,
            tab: self.tab,
            instances: Some(self.instances.clone()),
        };
        if std::fs::create_dir_all(&self.root).is_ok()
            && let Ok(bytes) = serde_json::to_vec_pretty(&prefs)
        {
            let _ = platform::atomic_write(&self.root.join("ui-preferences.json"), &bytes);
        }
    }
    fn job(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        f: impl FnOnce(Sender<Event>) -> Result<Reply> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.status = label.into();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let install = self.install();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tx.clone())));
            let result = match result {
                Ok(r) => r.map_err(|e| format!("{e:#}")),
                Err(_) => Err(
                    "The operation stopped unexpectedly. Check Backups for a recovery journal."
                        .into(),
                ),
            };
            // A queue, host apply or root operation can start/stop players and
            // change allocations. Refresh off the UI thread, keeping the result
            // and reviewed selections intact when the snapshot is received.
            if let Some(install) = install
                && let Ok(snapshot) = discovery::snapshot(install)
            {
                let _ = tx.send(Event::Refreshed(snapshot));
            }
            let _ = tx.send(Event::Finished(Box::new(result)));
            ctx.request_repaint();
        });
    }
    fn log(&mut self, text: String) {
        let text = format!("{}  {text}", chrono::Local::now().format("%H:%M:%S"));
        self.logs.push(text.clone());
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
        let dir = self.root.join("logs");
        if std::fs::create_dir_all(&dir).is_ok() {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{}.log", chrono::Local::now().format("%Y-%m-%d"))))
            {
                let _ = writeln!(f, "{text}");
            }
        }
    }
    fn receive(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Refreshed(snapshot) => self.snapshot = Some(snapshot),
                Event::Log(s) => {
                    if s.starts_with(backup_cleanup::WARNING_PREFIX) {
                        self.backup_warning = Some(s.clone());
                    }
                    if self.busy {
                        self.status = s.clone();
                    }
                    self.log(s)
                }
                Event::Finished(r) => {
                    self.busy = false;
                    self.queue_control = None;
                    match *r {
                        Ok(reply) => {
                            self.status = "Ready".into();
                            match reply {
                                Reply::Scanned(s) => {
                                    if !s.instances.iter().any(|i| i.name == self.instance) {
                                        self.instance = s
                                            .instances
                                            .iter()
                                            .find(|i| self.instances.contains(&i.name))
                                            .or_else(|| {
                                                s.instances.iter().find(|i| {
                                                    s.processes.iter().any(|p| {
                                                        p.instance.as_ref() == Some(&i.name)
                                                    })
                                                })
                                            })
                                            .or_else(|| s.instances.first())
                                            .map(|i| i.name.clone())
                                            .unwrap_or_default();
                                    }
                                    self.instances
                                        .retain(|name| s.instances.iter().any(|i| &i.name == name));
                                    if !self.selection_initialized {
                                        if !self.instance.is_empty() {
                                            self.instances.insert(self.instance.clone());
                                        }
                                        self.selection_initialized = true;
                                    }
                                    self.invalidate_android();
                                    self.install_path =
                                        s.installation.install_dir.to_string_lossy().into();
                                    self.conf_path =
                                        s.installation.data_dir.to_string_lossy().into();
                                    self.snapshot = Some(s);
                                    self.refresh_backup_list();
                                    self.clear_preview();
                                    self.log("Installation inspected. No settings changed.".into());
                                }
                                Reply::Preview(p) => {
                                    let count = Self::change_count(&p);
                                    self.status = if !p.issues.is_empty() {
                                        format!(
                                            "{} ready; {} option(s) unavailable. Review the details before applying.",
                                            Self::changes(count),
                                            p.issues
                                                .iter()
                                                .map(|issue| issue.changes)
                                                .sum::<usize>()
                                        )
                                    } else if count == 0 {
                                        "Review complete. No changes needed.".into()
                                    } else {
                                        format!(
                                            "{} ready. Click Apply to make them.",
                                            Self::changes(count)
                                        )
                                    };
                                    self.log(format!(
                                        "Preview ready: {} operation(s)",
                                        p.operations.len()
                                    ));
                                    self.plan = Some(p);
                                    self.applied = None;
                                    self.scroll_to_preview = true;
                                }
                                Reply::Applied(report) => {
                                    self.status = report.summary();
                                    self.log(self.status.clone());
                                    self.plan = None;
                                    self.applied = Some(action_bar::AppliedChanges {
                                        report,
                                        time: chrono::Local::now().format("%H:%M").to_string(),
                                    });
                                    self.backups =
                                        backup_cleanup::list(&self.root).unwrap_or_default();
                                }
                                Reply::Packages(p) => {
                                    self.packages = Self::aggregate_packages(&p);
                                    self.selected = self
                                        .packages
                                        .iter()
                                        .filter(|p| {
                                            (p.recommended || self.maximum_selected)
                                                && p.enabled <= 1
                                        })
                                        .map(|p| p.name.clone())
                                        .collect();
                                    self.scans = p;
                                    self.clear_preview();
                                    let connected =
                                        self.scans.iter().filter(|s| s.error.is_none()).count();
                                    self.status = format!(
                                        "{connected}/{} instances scanned. Select apps, then Review.",
                                        self.scans.len()
                                    );
                                }
                                Reply::BatchPreview(plan) => {
                                    self.status = format!(
                                        "{} ready across {}. Review each result below.",
                                        Self::changes(plan.change_count()),
                                        Self::instance_count(plan.entries.len())
                                    );
                                    self.batch_plan = Some(plan);
                                    self.scroll_to_preview = true;
                                }
                                Reply::BatchApplied(report) => {
                                    self.status = report.summary();
                                    self.log(self.status.clone());
                                    self.clear_preview();
                                    self.invalidate_android();
                                    self.batch_report = Some(report);
                                    self.refresh_backup_list();
                                }
                                Reply::Network(r) => self.report = Some(r),
                                Reply::Backups(b) => {
                                    if b.iter().all(|record| {
                                        record.problem.is_none()
                                            && record.can_delete
                                            && record.status != "Deleting"
                                    }) {
                                        self.backup_warning = None;
                                    }
                                    self.backups = b;
                                }
                                Reply::RootInfo(info) => self.root_info = Some(info),
                                Reply::RootStatus(status) => self.root_status = Some(status),
                                Reply::RootBackups(backups) => self.root_backups = backups,
                                Reply::Done(s) => {
                                    self.status = s.clone();
                                    self.log(s);
                                    self.plan = None;
                                    self.applied = None;
                                    self.backups =
                                        backup_cleanup::list(&self.root).unwrap_or_default();
                                }
                            }
                        }
                        Err(e) => {
                            self.root_status = None;
                            self.log(format!("ERROR: {e}"));
                            self.status = "Action needs attention".into();
                            self.error = Some(e);
                        }
                    }
                }
            }
        }
    }
    fn scan(&mut self, ctx: &egui::Context) {
        let install = self.install_path.clone();
        let conf = self.conf_path.clone();
        let root = self.root.clone();
        self.job(
            ctx,
            "Inspecting installation and maintaining backups…",
            move |tx| {
                if let Err(error) = backup_cleanup::prune(&root, |s| {
                    let _ = tx.send(Event::Log(s));
                }) {
                    let _ = tx.send(Event::Log(format!(
                        "{} {error:#}",
                        backup_cleanup::WARNING_PREFIX
                    )));
                }
                let i = (!install.trim().is_empty()).then(|| PathBuf::from(install.trim()));
                let c = (!conf.trim().is_empty()).then(|| PathBuf::from(conf.trim()));
                Ok(Reply::Scanned(discovery::snapshot(discovery::select(
                    i.as_deref(),
                    c.as_deref(),
                )?)?))
            },
        );
    }
    fn refresh_backup_list(&mut self) {
        match backup_cleanup::list(&self.root) {
            Ok(backups) => self.backups = backups,
            Err(error) => {
                let warning = format!("{} {error:#}", backup_cleanup::WARNING_PREFIX);
                self.log(warning.clone());
                self.backup_warning = Some(warning);
            }
        }
    }
    fn install(&self) -> Option<Installation> {
        self.snapshot.as_ref().map(|s| s.installation.clone())
    }
    fn preview_host(&mut self, ctx: &egui::Context, cloud: bool) {
        if let Some(snapshot) = self.snapshot.clone() {
            let instances = if cloud {
                vec![self.instance.clone()]
            } else {
                self.instances.iter().cloned().collect()
            };
            let options = if cloud {
                HostOptions {
                    ads: false,
                    telemetry: false,
                    smart_downloads: false,
                    cloud: true,
                    remove_x: self.options.remove_x,
                    remove_services: self.options.remove_services,
                    ..HostOptions::default()
                }
            } else {
                self.options.clone()
            };
            self.job(
                ctx,
                "Preparing exact changes and checking files…",
                move |_| {
                    Ok(Reply::Preview(engine::host_plan_many(
                        &snapshot, &instances, &options,
                    )?))
                },
            );
        }
    }
    fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        let compact =
            ui.ctx().content_rect().width() < 930.0 && ui.ctx().content_rect().height() < 340.0;
        if compact {
            // The compact navigation already names the page. Preserve room for
            // instance selection and settings at 200% text in a short window.
            ui.label(RichText::new(subtitle).small().color(MUTED));
        } else {
            ui.heading(title);
            ui.label(RichText::new(subtitle).color(MUTED));
        }
        ui.add_space(if compact { 6.0 } else { 14.0 });
    }
    fn overview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "A quieter BlueStacks.",
            "Keep the Android emulator. Choose what runs around it.",
        );
        if let Some(s) = self.snapshot.clone() {
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("BLUESTACKS", s.installation.version.clone()),
                    ("ANDROID INSTANCES", s.instances.len().to_string()),
                    (
                        "YOUR PC",
                        format!("{} threads / {} GB", s.cpu_count, s.ram_mb / 1024),
                    ),
                ] {
                    egui::Frame::group(ui.style())
                        .inner_margin(18.0)
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(140.0_f32.min(ui.available_width()));
                                ui.label(RichText::new(label).size(11.0).color(MUTED));
                                ui.add(
                                    egui::Label::new(RichText::new(value).size(22.0).strong())
                                        .wrap(),
                                );
                            });
                        });
                }
            });
            ui.add_space(18.0);
            for (number, title, detail) in [
                (
                    "01",
                    "Choose your instances",
                    "Check the instances to clean up in Debloat & tune. The picker here starts one instance.",
                ),
                (
                    "02",
                    "Preview your changes",
                    "Debloat & tune lists the exact settings before applying them.",
                ),
                (
                    "03",
                    "Apply the reviewed changes",
                    "Host changes close BlueStacks. Android apps can start and close stopped instances one at a time.",
                ),
            ] {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(number).size(21.0).color(ACCENT));
                    ui.vertical(|ui| {
                        ui.strong(title);
                        ui.label(RichText::new(detail).color(MUTED));
                    });
                });
                ui.add_space(5.0);
            }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Choose debloat options").clicked() { self.tab = Tab::Host; self.plan = None;
                                    self.applied = None; }
                if ui.button("Start selected instance").clicked() { let install = s.installation.clone(); let name = self.instance.clone(); self.job(ctx, "Starting BlueStacks…", move |_| { discovery::launch(&install, &name)?; Ok(Reply::Done("BlueStacks is starting. Wait for the Android home screen before scanning apps.".into())) }); }
                if ui.button("Stop BlueStacks").clicked() { let install = s.installation.clone(); self.job(ctx, "Stopping BlueStacks and its companions…", move |tx| { rooting::stop_players(&install, &mut |line| { let _ = tx.send(Event::Log(line)); })?; Ok(Reply::Done("BlueStacks and its companions are stopped.".into())) }); }
            });
            ui.add_space(18.0);
            egui::CollapsingHeader::new("Installation details").show(ui, |ui| {
                ui.label(format!("Program: {}", s.installation.install_dir.display()));
                ui.label(format!("Data: {}", s.installation.data_dir.display()));
                for p in &s.processes {
                    ui.label(format!(
                        "{} • PID {} • {} MB • {}",
                        p.name,
                        p.pid,
                        p.memory_mb,
                        p.instance.as_deref().unwrap_or("helper / unknown instance")
                    ));
                }
            });
        } else {
            ui.label(
                "Select custom paths below if automatic discovery could not find one installation.",
            );
        }
        ui.add_space(12.0);
        egui::CollapsingHeader::new("Custom installation paths").show(ui, |ui| {
            ui.label("Program folder (contains HD-Player.exe)");
            ui.text_edit_singleline(&mut self.install_path);
            ui.label("Data folder (contains bluestacks.conf)");
            ui.text_edit_singleline(&mut self.conf_path);
            if ui.button("Inspect these folders").clicked() {
                self.snapshot = None;
                self.plan = None;
                self.applied = None;
                self.scan(ctx);
            }
            if ui.button("Detect from registry again").clicked() {
                self.install_path.clear();
                self.conf_path.clear();
                self.snapshot = None;
                self.plan = None;
                self.applied = None;
                self.scan(ctx);
            }
        });
    }
    fn cloud(&mut self, ui: &mut egui::Ui) {
        Self::heading(
            ui,
            "Keep local. Remove the cloud.",
            "Remove BlueStacks X separately from your Android emulator.",
        );
        egui::Frame::group(ui.style()).inner_margin(18.0).show(ui, |ui| {
            if ui.checkbox(&mut self.options.remove_x, "Remove BlueStacks X / Store").changed() { self.plan = None;
                                    self.applied = None; }
            ui.label(RichText::new("Removes its active folder, matching startup entries, registrations and shortcuts. Turns off the emulator's cloud, rewards and AI integrations.").color(MUTED));
            ui.add_space(8.0); if ui.checkbox(&mut self.options.remove_services, "Also remove the separate BlueStacks Services app").changed() { self.plan = None;
                                    self.applied = None; }
            ui.label(RichText::new("The cloud companion app is separate from the emulator's VM services and drivers.").color(MUTED));
        });
        ui.add_space(12.0);
        ui.strong("Your local emulator stays installed");
        ui.label("The App Player, Multi-instance Manager, Android disks, games and saves remain available. Launch the emulator from this app or its BlueStacks 5 shortcut.");
        ui.label(RichText::new("Recovery copies keep the original cloud folders on the same drive. Removal stops active components but initially does not reclaim their disk space. Updates can reinstall them.").color(MUTED));
        ui.add_space(12.0);
        self.preview_details(ui);
    }
    fn network(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "BlueStacks network controls",
            "Target BlueStacks advertising and launcher traffic while keeping app services available.",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Capture network snapshot").clicked()
                && let Some(install) = self.install()
            {
                self.job(
                    ctx,
                    "Reading BlueStacks TCP connections and domains…",
                    move |_| Ok(Reply::Network(network::report(&install)?)),
                );
            }
            if ui.button("Export report").clicked()
                && let Some(install) = self.install()
            {
                self.job(ctx, "Saving network report…", move |_| {
                    Ok(Reply::Done(format!(
                        "Network report: {}",
                        network::export(&install)?.display()
                    )))
                });
            }
        });
        if let Some(r) = &self.report {
            ui.label(RichText::new(&r.explanation).color(MUTED));
            ui.label(format!(
                "Captured {} • {} TCP connection(s)",
                r.sampled_at,
                r.connections.len()
            ));
            egui::ScrollArea::vertical()
                .id_salt("connections")
                .max_height(210.0)
                .show(ui, |ui| {
                    egui::Grid::new("tcp")
                        .striped(true)
                        .min_col_width(35.0)
                        .show(ui, |ui| {
                            ui.strong("PID");
                            ui.strong("Remote IP");
                            ui.strong("Port");
                            ui.end_row();
                            for c in &r.connections {
                                ui.label(c.pid.to_string());
                                ui.monospace(&c.remote_ip);
                                ui.label(c.remote_port.to_string());
                                ui.end_row();
                            }
                        });
                });
            egui::CollapsingHeader::new("Domains in recent BlueStacks logs").show(ui, |ui| {
                for d in &r.log_domains {
                    ui.monospace(d);
                }
            });
        }
        ui.add_space(12.0);
        ui.strong("BlueStacks advertising endpoints");
        for domain in network::DOMAINS {
            ui.monospace(*domain);
        }
        ui.label(RichText::new("These are the only hostnames on the blocklist. Google Play, Google services and game domains remain allowed. Enable the PC option under Debloat & tune, or use the per-instance option below.").color(MUTED));
        ui.add_space(16.0);
        ui.separator();
        ui.strong("Advanced Android filtering (Magisk root required)");
        if ui
            .checkbox(
                &mut self.root_hosts,
                "Block these BlueStacks ad endpoints inside this instance",
            )
            .changed()
        {
            self.plan = None;
            self.applied = None;
        }
        if ui
            .checkbox(
                &mut self.root_isolate,
                "Block the BlueStacks launcher's Internet access",
            )
            .changed()
        {
            self.plan = None;
            self.applied = None;
        }
        ui.label(RichText::new("Launcher isolation blocks new online recommendations, including the home screen's 'Popular games to play' row, and the launcher's search and store connections. Cached tiles may remain visible. Games and Google Play keep their own network access. Restart Android to refresh the launcher and activate hosts filtering; removing the hosts overlay also requires a restart.").color(MUTED));
        self.preview_details(ui);
    }
    fn preview_details(&mut self, ui: &mut egui::Ui) {
        self.android_results(ui);
        if let Some(applied) = &self.applied
            && !applied.report.issues.is_empty()
        {
            ui.group(|ui| {
                ui.strong("Some options need attention");
                ui.label("Successful independent changes remain applied. Review again to retry the remaining options.");
                egui::ScrollArea::vertical().id_salt("apply_issues").max_height(150.0).show(ui, |ui| {
                    for issue in &applied.report.issues {
                        ui.add(egui::Label::new(RichText::new(format!("{}: {}", issue.label, issue.error)).color(Color32::from_rgb(244, 204, 128))).wrap());
                    }
                });
            });
        }
        if let Some(plan) = &self.plan {
            ui.add_space(14.0);
            ui.separator();
            let heading = ui.strong(format!(
                "Review: {} pending",
                Self::changes(Self::change_count(plan))
            ));
            if self.scroll_to_preview {
                heading.scroll_to_me(Some(egui::Align::Min));
                self.scroll_to_preview = false;
            }
            ui.label("This review has made no changes. Apply using the fixed bottom bar.");
            if !plan.issues.is_empty() {
                ui.strong("Unavailable options will be skipped; other changes can still apply.");
                for issue in &plan.issues {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}: {}", issue.label, issue.error))
                                .color(Color32::from_rgb(244, 204, 128)),
                        )
                        .wrap(),
                    );
                }
            }
            for op in &plan.operations {
                ui.label(RichText::new(&op.label).color(ACCENT));
                ui.collapsing(format!("Details: {}", op.label), |ui| {
                    if let Target::Config { edits, .. } = &op.target {
                        for e in edits {
                            ui.monospace(format!("{}: {} → {}", e.key, e.before, e.after));
                        }
                    } else {
                        ui.label(
                            RichText::new(format!("{:?}", op.target))
                                .small()
                                .color(MUTED),
                        );
                    }
                });
            }
            for note in &plan.notes {
                ui.label(RichText::new(note).small().color(MUTED));
            }
        }
    }
}
impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.render(ui);
    }
}
pub fn run() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 820.0])
            .with_min_inner_size([300.0, 260.0])
            .with_clamp_size_to_monitor_size(true),
        ..Default::default()
    };
    eframe::run_native(
        concat!("BlueStacks Debloat ", env!("CARGO_PKG_VERSION")),
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
    .context("Start native desktop window")
}
