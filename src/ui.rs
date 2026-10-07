use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
mod host_panel;
mod responsive;
mod root_panel;
use bluestacks_debloat::{
    adb::{Client, Package},
    discovery::{self, Installation, Snapshot},
    engine::{self, HostOptions, Performance},
    network, platform, rooting,
    transaction::{self, BackupInfo, Plan, Target},
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
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}
enum Reply {
    Scanned(Snapshot),
    Preview(Plan),
    Packages(Vec<Package>),
    Network(network::Report),
    Backups(Vec<BackupInfo>),
    Done(String),
    RootInfo(rooting::RootInfo),
    RootStatus(rooting::Verification),
    RootBackups(Vec<rooting::RootBackupInfo>),
}
enum Event {
    Log(String),
    Finished(Box<std::result::Result<Reply, String>>),
}
struct App {
    tab: Tab,
    snapshot: Option<Snapshot>,
    instance: String,
    install_path: String,
    conf_path: String,
    options: HostOptions,
    plan: Option<Plan>,
    packages: Vec<Package>,
    selected: BTreeSet<String>,
    animations: bool,
    report: Option<network::Report>,
    backups: Vec<BackupInfo>,
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
        let mut style = (*cc.egui_ctx.style()).clone();
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
        cc.egui_ctx.set_style(style);
        let (tx, rx) = mpsc::channel();
        let mut app = Self {
            tab: Tab::Overview,
            snapshot: None,
            instance: String::new(),
            install_path: String::new(),
            conf_path: String::new(),
            options: HostOptions::default(),
            plan: None,
            packages: vec![],
            selected: BTreeSet::new(),
            animations: true,
            report: None,
            backups: vec![],
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
            root: platform::state_dir(),
        };
        if let Ok(bytes) = std::fs::read(app.root.join("ui-preferences.json"))
            && let Ok(saved) = serde_json::from_slice::<Preferences>(&bytes)
        {
            app.instance = saved.instance;
            app.install_path = saved.install_path;
            app.conf_path = saved.conf_path;
            app.text_scale = saved.text_scale.clamp(1.0, 2.0);
            app.tab = saved.tab;
            cc.egui_ctx.set_zoom_factor(app.text_scale);
        }
        app.scan(&cc.egui_ctx);
        app
    }
    fn save_preferences(&self) {
        let prefs = Preferences {
            instance: self.instance.clone(),
            install_path: self.install_path.clone(),
            conf_path: self.conf_path.clone(),
            text_scale: self.text_scale,
            tab: self.tab,
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
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(tx.clone())));
            let result = match result {
                Ok(r) => r.map_err(|e| format!("{e:#}")),
                Err(_) => Err(
                    "The operation stopped unexpectedly. Check Backups for a recovery journal."
                        .into(),
                ),
            };
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
                Event::Log(s) => {
                    if self.busy {
                        self.status = s.clone();
                    }
                    self.log(s)
                }
                Event::Finished(r) => {
                    self.busy = false;
                    match *r {
                        Ok(reply) => {
                            self.status = "Ready".into();
                            match reply {
                                Reply::Scanned(s) => {
                                    if !s.instances.iter().any(|i| i.name == self.instance) {
                                        self.instance = s
                                            .instances
                                            .first()
                                            .map(|i| i.name.clone())
                                            .unwrap_or_default();
                                    }
                                    self.install_path =
                                        s.installation.install_dir.to_string_lossy().into();
                                    self.conf_path =
                                        s.installation.data_dir.to_string_lossy().into();
                                    self.snapshot = Some(s);
                                    self.plan = None;
                                    self.log("Installation inspected. No settings changed.".into());
                                }
                                Reply::Preview(p) => {
                                    self.log(format!(
                                        "Preview ready: {} operation(s)",
                                        p.operations.len()
                                    ));
                                    self.plan = Some(p);
                                }
                                Reply::Packages(p) => {
                                    self.selected = p
                                        .iter()
                                        .filter(|p| {
                                            (p.recommended || self.maximum_selected)
                                                && p.enabled <= 1
                                        })
                                        .map(|p| p.name.clone())
                                        .collect();
                                    self.packages = p;
                                    self.plan = None;
                                }
                                Reply::Network(r) => self.report = Some(r),
                                Reply::Backups(b) => self.backups = b,
                                Reply::RootInfo(info) => self.root_info = Some(info),
                                Reply::RootStatus(status) => self.root_status = Some(status),
                                Reply::RootBackups(backups) => self.root_backups = backups,
                                Reply::Done(s) => {
                                    self.status = s.clone();
                                    self.log(s);
                                    self.plan = None;
                                    self.backups =
                                        transaction::backups(&self.root).unwrap_or_default();
                                }
                            }
                        }
                        Err(e) => {
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
        self.job(ctx, "Inspecting installation…", move |_| {
            let i = (!install.trim().is_empty()).then(|| PathBuf::from(install.trim()));
            let c = (!conf.trim().is_empty()).then(|| PathBuf::from(conf.trim()));
            Ok(Reply::Scanned(discovery::snapshot(discovery::select(
                i.as_deref(),
                c.as_deref(),
            )?)?))
        });
    }
    fn install(&self) -> Option<Installation> {
        self.snapshot.as_ref().map(|s| s.installation.clone())
    }
    fn preview_host(&mut self, ctx: &egui::Context, cloud: bool) {
        if let Some(snapshot) = self.snapshot.clone() {
            let instance = self.instance.clone();
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
                    Ok(Reply::Preview(engine::host_plan(
                        &snapshot, &instance, &options,
                    )?))
                },
            );
        }
    }
    fn heading(ui: &mut egui::Ui, title: &str, subtitle: &str) {
        ui.heading(title);
        ui.label(RichText::new(subtitle).color(MUTED));
        ui.add_space(14.0);
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
                    "Choose an instance",
                    "Select the Android instance at the top or in the sidebar.",
                ),
                (
                    "02",
                    "Preview your changes",
                    "Debloat & tune lists the exact settings before applying them.",
                ),
                (
                    "03",
                    "Apply, then start BlueStacks",
                    "Host changes need BlueStacks closed. Android changes need it running.",
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
                if ui.button("Choose debloat options").clicked() { self.tab = Tab::Host; self.plan = None; }
                if ui.button("Start selected instance").clicked() { let install = s.installation.clone(); let name = self.instance.clone(); self.job(ctx, "Starting BlueStacks…", move |_| { discovery::launch(&install, &name)?; Ok(Reply::Done("BlueStacks is starting. Wait for the Android home screen before scanning apps.".into())) }); }
                if ui.button("Request normal close").clicked() { let install = s.installation.clone(); self.job(ctx, "Requesting BlueStacks to close…", move |_| { let pids = discovery::processes(&install)?.into_iter().filter(|p| p.name.eq_ignore_ascii_case("HD-Player.exe") && p.instance.is_some()).map(|p| p.pid).collect::<Vec<_>>(); platform::close_windows(&pids); Ok(Reply::Done("Close requested. Complete BlueStacks' exit dialog and close the Multi-instance Manager.".into())) }); }
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
                self.scan(ctx);
            }
            if ui.button("Detect from registry again").clicked() {
                self.install_path.clear();
                self.conf_path.clear();
                self.snapshot = None;
                self.plan = None;
                self.scan(ctx);
            }
        });
    }
    fn android(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "Android apps",
            "Start the selected instance. Enable ADB in BlueStacks Settings > Advanced.",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Scan Android apps").clicked()
                && let Some(install) = self.install()
            {
                let name = self.instance.clone();
                self.job(ctx, "Connecting to Android…", move |_| {
                    Ok(Reply::Packages(engine::guest_scan(&install, &name)?.1))
                });
            }
            if ui.button("Save Android screenshot").clicked()
                && let Some(install) = self.install()
            {
                let name = self.instance.clone();
                let dir = self.root.join("screenshots");
                self.job(ctx, "Capturing Android screen…", move |_| {
                    std::fs::create_dir_all(&dir)?;
                    let path = dir.join(format!(
                        "{name}-{}.png",
                        chrono::Utc::now().format("%Y%m%d-%H%M%S")
                    ));
                    Client::connect(&install, &name)?.screenshot(&path)?;
                    Ok(Reply::Done(format!("Screenshot saved: {}", path.display())))
                });
            }
        });
        ui.add_space(10.0);
        if self.packages.is_empty() {
            ui.label(
                RichText::new(
                    "Scan to find reviewed optional packages installed in this instance.",
                )
                .color(MUTED),
            );
        }
        for p in &self.packages {
            let mut selected = self.selected.contains(&p.name);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(p.enabled <= 1, egui::Checkbox::new(&mut selected, &p.name))
                    .changed()
                {
                    if selected {
                        self.selected.insert(p.name.clone());
                    } else {
                        self.selected.remove(&p.name);
                    }
                    self.plan = None;
                }
                if p.enabled > 1 {
                    ui.label(RichText::new("already disabled").color(ACCENT));
                }
            });
            ui.label(RichText::new(&p.reason).small().color(MUTED));
        }
        ui.add_space(8.0);
        if ui
            .checkbox(
                &mut self.animations,
                "Shorten Android UI animations to 0.5×",
            )
            .changed()
        {
            self.plan = None;
        }
        ui.label(RichText::new("Games, the launcher, Google Play, billing and accounts are excluded. Disabling keeps app data and can be undone.").color(MUTED));
        if ui.button("Preview Android changes").clicked()
            && let Some(install) = self.install()
        {
            let name = self.instance.clone();
            let selected = self.selected.iter().cloned().collect::<Vec<_>>();
            let animations = self.animations;
            self.job(ctx, "Reading current Android settings…", move |_| {
                Ok(Reply::Preview(engine::guest_plan(
                    &install, &name, &selected, animations,
                )?))
            });
        }
        self.preview(ui, ctx);
    }
    fn cloud(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "Keep local. Remove the cloud.",
            "Remove BlueStacks X separately from your Android emulator.",
        );
        egui::Frame::group(ui.style()).inner_margin(18.0).show(ui, |ui| {
            if ui.checkbox(&mut self.options.remove_x, "Remove BlueStacks X / Store").changed() { self.plan = None; }
            ui.label(RichText::new("Removes its active folder, matching startup entries, registrations and shortcuts. Turns off the emulator's cloud, rewards and AI integrations.").color(MUTED));
            ui.add_space(8.0); if ui.checkbox(&mut self.options.remove_services, "Also remove the separate BlueStacks Services app").changed() { self.plan = None; }
            ui.label(RichText::new("The cloud companion app is separate from the emulator's VM services and drivers.").color(MUTED));
        });
        ui.add_space(12.0);
        ui.strong("Your local emulator stays installed");
        ui.label("The App Player, Multi-instance Manager, Android disks, games and saves remain available. Launch the emulator from this app or its BlueStacks 5 shortcut.");
        ui.label(RichText::new("Recovery copies keep the original cloud folders on the same drive. Removal stops active components but initially does not reclaim their disk space. Updates can reinstall them.").color(MUTED));
        ui.add_space(12.0);
        if ui.button("Preview cloud removal").clicked() {
            self.preview_host(ctx, true);
        }
        self.preview(ui, ctx);
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
        }
        if ui
            .checkbox(
                &mut self.root_isolate,
                "Block the BlueStacks launcher's Internet access",
            )
            .changed()
        {
            self.plan = None;
        }
        ui.label(RichText::new("Launcher isolation stops its online recommendations, search and store functions. Games and Google Play keep their own network access. Restart Android to refresh the launcher. Hosts filtering takes effect after an Android restart; restore also needs a restart to remove that overlay.").color(MUTED));
        if ui.button("Preview root network controls").clicked()
            && let Some(install) = self.install()
        {
            let name = self.instance.clone();
            let hosts = self.root_hosts;
            let isolate = self.root_isolate;
            self.job(
                ctx,
                "Inspecting Magisk and Android network rules…",
                move |_| {
                    Ok(Reply::Preview(engine::root_plan(
                        &install, &name, hosts, isolate,
                    )?))
                },
            );
        }
        self.preview(ui, ctx);
    }
    fn backups(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "Recover your changes",
            "Each operation has its own verified backup and restore journal.",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh backups").clicked() {
                let root = self.root.clone();
                self.job(ctx, "Reading operation journals…", move |_| {
                    Ok(Reply::Backups(transaction::backups(&root)?))
                });
            }
            if ui.button("Open backup folder").clicked() {
                let _ = platform::open_folder(&self.root.join("backups"));
            }
        });
        ui.label(RichText::new("Close BlueStacks for host restores. Start the original instance for Android restores. Values modified afterwards are preserved and reported as conflicts.").color(MUTED));
        let mut restore = None;
        for b in &self.backups {
            egui::Frame::group(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(&b.title);
                            ui.label(
                                RichText::new(format!("{} • {}", b.created, b.status))
                                    .small()
                                    .color(MUTED),
                            );
                        });
                        if ui
                            .add_enabled(
                                b.status != "restored",
                                egui::Button::new("Restore this operation"),
                            )
                            .clicked()
                        {
                            restore = Some(b.path.clone());
                        }
                    });
                });
        }
        if let Some(path) = restore {
            let root = self.root.clone();
            self.job(
                ctx,
                "Restoring and verifying original values…",
                move |tx| {
                    transaction::restore(&path, &root, |s| {
                        let _ = tx.send(Event::Log(s));
                    })?;
                    Ok(Reply::Done("Restoration verified.".into()))
                },
            );
        }
        if self.backups.is_empty() {
            ui.label("No operation backups yet.");
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut apply = false;
        if let Some(plan) = &self.plan {
            ui.add_space(14.0);
            ui.separator();
            ui.strong(format!("Preview • {} operation(s)", plan.operations.len()));
            egui::ScrollArea::vertical()
                .id_salt("preview")
                .max_height(260.0)
                .show(ui, |ui| {
                    for op in &plan.operations {
                        ui.label(RichText::new(&op.label).color(ACCENT));
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
                    }
                    for note in &plan.notes {
                        ui.label(RichText::new(note).small().color(MUTED));
                    }
                });
            let admin = plan.guest.is_some() || platform::is_admin();
            if !admin {
                ui.label("Restart as administrator to apply host changes.");
            }
            apply = ui
                .add_enabled(
                    admin && !plan.operations.is_empty(),
                    egui::Button::new("Apply these changes").fill(Color32::from_rgb(35, 100, 91)),
                )
                .clicked();
            if plan.operations.is_empty() {
                ui.label(
                    RichText::new(
                        "Everything selected is already configured, or is absent in this build.",
                    )
                    .color(ACCENT),
                );
            }
        }
        if apply && let Some(plan) = self.plan.take() {
            let root = self.root.clone();
            self.job(ctx, "Applying and verifying changes…", move |tx| {
                let path = transaction::apply(plan, &root, |s| {
                    let _ = tx.send(Event::Log(s));
                })?;
                Ok(Reply::Done(format!(
                    "Changes verified. Recovery backup: {}",
                    path.display()
                )))
            });
        }
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.render(ctx);
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
        "BlueStacks Debloat",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
    .context("Start native desktop window")
}
