use super::*;
const TABS: &[(Tab, &str)] = &[
    (Tab::Overview, "Overview"),
    (Tab::Host, "Debloat & tune"),
    (Tab::Android, "Android apps"),
    (Tab::Cloud, "BlueStacks X"),
    (Tab::Network, "Network"),
    (Tab::Root, "Root BlueStacks"),
    (Tab::Backups, "Backups & restore"),
];
impl App {
    fn choose_tab(&mut self, tab: Tab, ctx: &egui::Context) {
        if self.tab != tab {
            self.plan = None;
        }
        self.tab = tab;
        if tab == Tab::Backups {
            let root = self.root.clone();
            self.job(ctx, "Reading backups…", move |_| {
                Ok(Reply::Backups(transaction::backups(&root)?))
            });
        }
    }
    fn instance_picker(&mut self, ui: &mut egui::Ui) {
        let old = self.instance.clone();
        egui::ComboBox::from_id_salt("instance")
            .width(ui.available_width().min(190.0))
            .selected_text(if self.instance.is_empty() {
                "Select an instance"
            } else {
                &self.instance
            })
            .show_ui(ui, |ui| {
                if let Some(s) = &self.snapshot {
                    for i in &s.instances {
                        ui.selectable_value(
                            &mut self.instance,
                            i.name.clone(),
                            format!("{} · {}", i.name, i.display_name),
                        );
                    }
                }
            });
        if old != self.instance {
            self.plan = None;
            self.packages.clear();
            self.selected.clear();
            self.root_info = None;
            self.root_status = None;
        }
    }
    pub(super) fn render(&mut self, ctx: &egui::Context) {
        self.receive();
        if ctx.input(|i| i.viewport().close_requested()) && !self.busy {
            self.save_preferences();
        }
        if !self.layout_initialized
            && let Some(m) = ctx.input(|i| i.viewport().monitor_size)
        {
            let size = egui::vec2(1120.0, 820.0)
                .min((m - egui::vec2(32.0, 90.0)).max(egui::vec2(300.0, 260.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            self.layout_initialized = true;
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
            if ctx.input(|i| i.viewport().close_requested()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        let narrow = ctx.content_rect().width() < 930.0;
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("B").size(26.0).strong().color(ACCENT));
                ui.label(
                    RichText::new("BlueStacks Debloat")
                        .size(if narrow { 18.0 } else { 21.0 })
                        .strong(),
                );
                if platform::is_admin() {
                    ui.label(RichText::new("Administrator").small().color(ACCENT));
                } else if ui
                    .add_enabled(!self.busy, egui::Button::new("Restart as administrator"))
                    .clicked()
                {
                    self.save_preferences();
                    match platform::elevate() {
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                egui::ComboBox::from_id_salt("text_size")
                    .selected_text(format!("Text {}%", (self.text_scale * 100.0).round()))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
                            if ui
                                .selectable_value(
                                    &mut self.text_scale,
                                    scale,
                                    format!("{}%", (scale * 100.0) as u32),
                                )
                                .changed()
                            {
                                ctx.set_zoom_factor(scale);
                            }
                        }
                    });
            });
            if narrow {
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let mut next = self.tab;
                        egui::ComboBox::from_id_salt("compact_nav")
                            .width(ui.available_width().min(160.0))
                            .selected_text(TABS.iter().find(|(t, _)| *t == self.tab).unwrap().1)
                            .show_ui(ui, |ui| {
                                for &(tab, label) in TABS {
                                    ui.selectable_value(&mut next, tab, label);
                                }
                            });
                        if next != self.tab {
                            self.choose_tab(next, ctx);
                        }
                        self.instance_picker(ui);
                    });
                });
            }
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.busy {
                    ui.spinner();
                } else {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.0, ACCENT);
                }
                ui.add(egui::Label::new(&self.status).wrap().truncate());
            });
            egui::CollapsingHeader::new("Activity and details").show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(110.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for l in &self.logs {
                            ui.add(egui::Label::new(RichText::new(l).monospace()).wrap());
                        }
                    });
            });
        });
        if !narrow {
            egui::SidePanel::left("navigation")
                .exact_width(214.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.add_space(12.0);
                            for &(tab, label) in TABS {
                                if ui
                                    .add_sized(
                                        [190.0, 36.0],
                                        egui::Button::new(label).selected(self.tab == tab),
                                    )
                                    .clicked()
                                {
                                    self.choose_tab(tab, ctx);
                                }
                            }
                            ui.add_space(20.0);
                            ui.label(RichText::new("ANDROID INSTANCE").size(11.0).color(MUTED));
                            self.instance_picker(ui);
                            if let Some(i) = self
                                .snapshot
                                .as_ref()
                                .and_then(|s| s.instances.iter().find(|i| i.name == self.instance))
                            {
                                ui.label(
                                    RichText::new(format!("{} cores · {} MB", i.cpus, i.ram_mb))
                                        .small()
                                        .color(MUTED),
                                );
                            }
                            if ui.button("Refresh installation").clicked() {
                                self.root_info = None;
                                self.root_status = None;
                                self.scan(ctx);
                            }
                            ui.label(
                                RichText::new("Preview first.\nRestore when needed.").color(MUTED),
                            );
                        });
                    });
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    if let Some(error) = self.error.clone() {
                        egui::Frame::group(ui.style())
                            .fill(Color32::from_rgb(67, 31, 36))
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.strong("This action could not finish");
                                ui.label(error.lines().next().unwrap_or(&error));
                                ui.collapsing("Details", |ui| {
                                    ui.add(egui::Label::new(&error).wrap());
                                });
                                if ui.small_button("Dismiss").clicked() {
                                    self.error = None;
                                }
                            });
                        ui.add_space(10.0);
                    }
                    ui.add_enabled_ui(!self.busy, |ui| match self.tab {
                        Tab::Overview => self.overview(ui, ctx),
                        Tab::Host => self.host(ui, ctx),
                        Tab::Android => self.android(ui, ctx),
                        Tab::Cloud => self.cloud(ui, ctx),
                        Tab::Network => self.network(ui, ctx),
                        Tab::Root => self.root_page(ui, ctx),
                        Tab::Backups => self.backups(ui, ctx),
                    });
                });
        });
    }
}
