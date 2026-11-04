use super::*;

impl App {
    pub(super) fn invalidate_android(&mut self) {
        self.scans.clear();
        self.packages.clear();
        self.selected.clear();
    }

    pub(super) fn aggregate_packages(scans: &[batch::InstanceScan]) -> Vec<Package> {
        let mut packages = std::collections::BTreeMap::<String, Package>::new();
        for package in scans.iter().flat_map(|s| &s.packages) {
            packages
                .entry(package.name.clone())
                .and_modify(|p| p.enabled = p.enabled.min(package.enabled))
                .or_insert_with(|| package.clone());
        }
        packages.into_values().collect()
    }

    pub(super) fn instance_selection(&mut self, ui: &mut egui::Ui) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        let instances = snapshot.instances.clone();
        let running = snapshot
            .processes
            .iter()
            .filter_map(|p| p.instance.clone())
            .collect::<BTreeSet<_>>();
        let before = self.instances.clone();
        egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
            ui.strong(format!("Apply to {} of {} instances", self.instances.len(), instances.len()));
            ui.horizontal_wrapped(|ui| {
                if ui.small_button(format!("Select all {}", instances.len())).clicked() { self.instances = instances.iter().map(|i| i.name.clone()).collect(); }
                if ui.small_button("Clear selection").clicked() { self.instances.clear(); }
                if ui.small_button("Refresh instances").clicked() { self.scan(&ui.ctx().clone()); }
            });
            if instances.len() > 6 {
                ui.add(egui::TextEdit::singleline(&mut self.instance_filter).hint_text("Find an instance…").desired_width(ui.available_width()));
            }
            egui::ScrollArea::vertical().id_salt("instance_selection").max_height(190.0).show(ui, |ui| {
                let filter = self.instance_filter.to_lowercase();
                for instance in &instances {
                    let label = format!("{} · {} · {}", instance.display_name, instance.name, android_version(&instance.name));
                    if instances.len() > 6 && !label.to_lowercase().contains(&filter) { continue; }
                    let mut selected = self.instances.contains(&instance.name);
                    ui.horizontal_wrapped(|ui| {
                        if ui.checkbox(&mut selected, label).changed() {
                            if selected { self.instances.insert(instance.name.clone()); } else { self.instances.remove(&instance.name); }
                        }
                        if running.contains(&instance.name) { ui.label(RichText::new("Running").small().color(ACCENT)); }
                    });
                }
            });
            if self.instances.is_empty() { ui.colored_label(Color32::from_rgb(244, 204, 128), "Select at least one instance to continue."); }
            if self.tab == Tab::Host {
                ui.label(RichText::new("Instance options apply to this selection. Shared PC options apply once. Applying closes all BlueStacks instances.").small().color(MUTED));
            } else {
                ui.label(RichText::new("This selection is shared with Debloat & tune. Android changes get a separate result and recovery copy for each instance.").small().color(MUTED));
            }
        });
        if self.instances != before {
            self.clear_preview();
            self.invalidate_android();
            self.error = None;
        }
        ui.add_space(12.0);
    }

    pub(super) fn android(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "Android apps",
            "Select instances, scan their apps, then review and apply the queue.",
        );
        self.instance_selection(ui);
        if ui
            .checkbox(
                &mut self.start_offline,
                "Start stopped instances automatically",
            )
            .changed()
        {
            self.clear_preview();
        }
        ui.label(RichText::new("Runs one queued instance at a time to limit memory use. Instances started here close after each step. Already running instances stay open.").small().color(MUTED));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!self.instances.is_empty(), egui::Button::new(if self.start_offline { "Start & scan selected instances" } else { "Scan selected instances" })).clicked()
                && let Some(install) = self.install()
            {
                let names = self.instances.iter().cloned().collect::<Vec<_>>();
                let start = self.start_offline;
                let control = batch::Control::default();
                self.queue_control = Some(control.clone());
                self.clear_preview();
                self.invalidate_android();
                self.job(ctx, "Scanning Android instances…", move |tx| {
                    Ok(Reply::Packages(batch::scan(&install, &names, start, &control, |s| { let _ = tx.send(Event::Log(s)); })?))
                });
            }
            if ui.small_button("ADB setup").clicked() {
                self.tab = Tab::Host;
                self.options.enable_adb = true;
                self.clear_preview();
                self.status = "Enable local Android tools is selected. Review and apply, then return to Android apps.".into();
            }
            if self.instances.len() == 1 && ui.small_button("Save screenshot").clicked()
                && let Some(install) = self.install()
                && let Some(name) = self.instances.first().cloned()
            {
                let dir = self.root.join("screenshots");
                self.job(ctx, "Capturing Android screen…", move |_| {
                    std::fs::create_dir_all(&dir)?;
                    let path = dir.join(format!("{name}-{}.png", chrono::Utc::now().format("%Y%m%d-%H%M%S")));
                    Client::connect(&install, &name)?.screenshot(&path)?;
                    Ok(Reply::Done(format!("Screenshot saved: {}", path.display())))
                });
            }
        });
        for scan in &self.scans {
            if let Some(error) = &scan.error {
                ui.colored_label(
                    Color32::from_rgb(244, 204, 128),
                    format!("{}: {error}", scan.instance),
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "{}: scanned · {} optional apps found",
                        scan.instance,
                        scan.packages.len()
                    ))
                    .color(ACCENT),
                );
            }
        }
        ui.add_space(12.0);
        if self.scans.is_empty() {
            ui.label("Scan first to see which optional apps are installed. ADB must be enabled in BlueStacks Settings > Advanced, or use ADB setup above.");
        } else if self.packages.is_empty() && self.scans.iter().any(|s| s.error.is_none()) {
            ui.label("Scan complete. No reviewed optional apps were found. Animation settings are still available.");
        }
        if !self.packages.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Recommended apps").clicked() {
                    self.selected = self
                        .packages
                        .iter()
                        .filter(|p| p.recommended && p.enabled <= 1)
                        .map(|p| p.name.clone())
                        .collect();
                    self.clear_preview();
                }
                if ui.small_button("Clear apps").clicked() {
                    self.selected.clear();
                    self.clear_preview();
                }
            });
        }
        let mut changed = false;
        for package in &self.packages {
            let present = self
                .scans
                .iter()
                .filter(|s| s.packages.iter().any(|p| p.name == package.name))
                .count();
            let enabled = self
                .scans
                .iter()
                .filter(|s| {
                    s.packages
                        .iter()
                        .any(|p| p.name == package.name && p.enabled <= 1)
                })
                .count();
            let mut selected = self.selected.contains(&package.name);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        enabled > 0,
                        egui::Checkbox::new(&mut selected, &package.name),
                    )
                    .changed()
                {
                    if selected {
                        self.selected.insert(package.name.clone());
                    } else {
                        self.selected.remove(&package.name);
                    }
                    changed = true;
                }
                ui.label(
                    RichText::new(if enabled == 0 {
                        "Already disabled".into()
                    } else {
                        format!("Enabled on {enabled} of {present} installed")
                    })
                    .small()
                    .color(if enabled == 0 { ACCENT } else { MUTED }),
                );
            });
            ui.label(RichText::new(&package.reason).small().color(MUTED));
        }
        if changed {
            self.clear_preview();
        }
        ui.add_space(8.0);
        if ui
            .checkbox(
                &mut self.animations,
                "Shorten Android UI animations to 0.5×",
            )
            .changed()
        {
            self.clear_preview();
        }
        ui.label(RichText::new("Only installed apps are changed on each instance. Games, the launcher, Google Play, billing and accounts are excluded. Disabling keeps app data and can be undone.").small().color(MUTED));
        self.preview_details(ui);
    }

    pub(super) fn android_results(&mut self, ui: &mut egui::Ui) {
        if let Some(batch) = &self.batch_plan {
            ui.separator();
            let heading = ui.strong(format!(
                "Review: {} across {}",
                Self::changes(batch.change_count()),
                Self::instance_count(batch.entries.len())
            ));
            if self.scroll_to_preview {
                heading.scroll_to_me(Some(egui::Align::Min));
                self.scroll_to_preview = false;
            }
            ui.label("No app settings changed during review. Apply runs the ready instances in order; an unavailable instance does not block the others.");
            for entry in &batch.entries {
                ui.push_id(&entry.instance, |ui| {
                    if let Some(error) = &entry.error {
                        ui.colored_label(
                            Color32::from_rgb(244, 204, 128),
                            format!("{} · Unavailable: {error}", entry.instance),
                        );
                    }
                    if let Some(plan) = &entry.plan {
                        ui.collapsing(
                            format!(
                                "{} · {} ready",
                                entry.instance,
                                Self::changes(Self::change_count(plan))
                            ),
                            |ui| {
                                for operation in &plan.operations {
                                    ui.label(&operation.label);
                                }
                                for issue in &plan.issues {
                                    ui.label(format!("{}: {}", issue.label, issue.error));
                                }
                            },
                        );
                    }
                });
            }
        }
        if let Some(report) = &self.batch_report {
            ui.separator();
            ui.strong(report.summary());
            for entry in &report.instances {
                ui.group(|ui| {
                    ui.strong(&entry.instance);
                    if let Some(error) = &entry.error {
                        ui.colored_label(Color32::from_rgb(244, 204, 128), error);
                    }
                    if let Some(result) = &entry.report {
                        ui.label(result.summary());
                        for issue in &result.issues {
                            ui.label(format!("{}: {}", issue.label, issue.error));
                        }
                        if let Some(backup) = &result.backup
                            && ui.small_button("Open this recovery copy").clicked()
                            && let Err(error) = platform::open_folder(backup)
                        {
                            self.error = Some(format!("{error:#}"));
                        }
                    }
                });
            }
        }
    }
}

fn android_version(name: &str) -> &'static str {
    match name.split('_').next().unwrap_or(name) {
        "Nougat32" => "Android 7 · 32-bit",
        "Nougat64" => "Android 7 · 64-bit",
        "Pie64" => "Android 9",
        "Rvc64" => "Android 11",
        "Tiramisu64" => "Android 13",
        _ => "Android",
    }
}
