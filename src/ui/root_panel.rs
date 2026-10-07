use super::*;
impl App {
    pub(super) fn root_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Root BlueStacks");
        let supported = self
            .instance
            .split('_')
            .next()
            .is_some_and(|s| ["Pie64", "Rvc64", "Tiramisu64"].contains(&s));
        if !supported {
            ui.label("Choose an Android 9, 11 or 13 (64-bit) instance. Other versions can still use the debloat tools.");
            return;
        }
        if self.root_info.is_none()
            && !self.busy
            && self.error.is_none()
            && let Some(install) = self.install()
        {
            let name = self.instance.clone();
            self.job(ctx, "Checking root support…", move |_| {
                Ok(Reply::RootInfo(rooting::info(&install, &name)?))
            });
        }
        if let Some(info) = self.root_info.clone() {
            let display = self
                .snapshot
                .as_ref()
                .and_then(|s| s.instances.iter().find(|i| i.name == info.instance))
                .map(|i| i.display_name.as_str())
                .unwrap_or(&info.instance);
            egui::Frame::group(ui.style())
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!("{display} · {}", info.instance))
                            .size(18.0)
                            .strong(),
                    );
                    ui.label(format!(
                        "Kitsune Mask 31 · Recovery copies ~{:.1} GiB",
                        info.backup_bytes as f64 / 1073741824.0
                    ));
                    if let Some(report) = &self.root_status {
                        let label = if report.root && report.complete {
                            "Magisk root verified"
                        } else if report.root {
                            "Root needs repair"
                        } else {
                            "No verified Magisk root in this instance"
                        };
                        ui.label(RichText::new(label).strong().color(
                            if report.root && report.complete {
                                ACCENT
                            } else {
                                Color32::from_rgb(238, 185, 106)
                            },
                        ));
                    } else {
                        ui.label(RichText::new("Ready to check or install Magisk.").color(MUTED));
                    }
                });
            ui.add_space(10.0);
            ui.label(RichText::new("Save game progress first. Rooting closes BlueStacks and restarts this instance. CPU cores and RAM stay as configured.").color(MUTED));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        platform::is_admin(),
                        egui::Button::new(if self.root_repair {
                            "Repair root now"
                        } else {
                            "Root this instance"
                        })
                        .fill(Color32::from_rgb(35, 100, 91)),
                    )
                    .clicked()
                {
                    let state = self.root.clone();
                    let repair = self.root_repair;
                    let info = info.clone();
                    self.root_status = None;
                    self.job(ctx, "Starting the guided root operation…", move |tx| {
                        Ok(Reply::RootStatus(rooting::install(
                            info,
                            repair,
                            &state,
                            |s| {
                                let _ = tx.send(Event::Log(s));
                            },
                        )?))
                    });
                }
                if ui.button("Start & check root").clicked() {
                    let info = info.clone();
                    self.job(
                        ctx,
                        "Checking Magisk, root helpers and boot state…",
                        move |tx| {
                            Ok(Reply::RootStatus(rooting::verify(&info, &mut |s| {
                                let _ = tx.send(Event::Log(s));
                            })?))
                        },
                    );
                }
            });
            if !platform::is_admin() {
                ui.label("Use Restart as administrator in the top bar to install or remove root.");
            }
            ui.add_space(10.0);
            ui.collapsing("What happens during rooting",|ui|{
                for line in ["1. Verify payloads and disk space; save full recovery copies.","2. Prepare the system, install Magisk and initialize it.","3. Remove temporary helpers, restart and verify Magisk-only root."]{ui.label(line);}
                ui.label(format!("Shared system disk: {}. Only the selected instance receives its root-enable flag.",info.shared_instances.join(", ")));
                ui.label("All required root tools are included. Additional temporary disk space is checked before changes.");
            });
            egui::CollapsingHeader::new("Repair, remove root, and advanced recovery").show(ui,|ui|{
                ui.checkbox(&mut self.root_repair,"Reinstall and repair even if Magisk is already working");
                ui.label(RichText::new("Unroot removes Magisk and its modules from this instance. Root-dependent network filtering will stop. Other instances keep their own root.").color(MUTED));
                if ui.add_enabled(platform::is_admin(),egui::Button::new("Unroot this instance")).clicked(){let state=self.root.clone();let info=info.clone();self.root_status=None;self.job(ctx,"Removing root with a recovery copy…",move |tx|{rooting::unroot(info,&state,|s|{let _=tx.send(Event::Log(s));})?;Ok(Reply::Done("Selected instance unrooted and checked.".into()))});}
                ui.separator();ui.strong("Restore all shared root files");ui.label("This affects every installed Android version and requires each original system backup plus the original player backup. Root and Magisk modules are removed across the installation.");
                if ui.add_enabled(platform::is_admin(),egui::Button::new("Restore all original shared root files")).clicked(){let state=self.root.clone();let install=info.installation.clone();self.root_status=None;self.job(ctx,"Restoring all shared root files…",move |tx|{rooting::full_unroot(install,&state,|s|{let _=tx.send(Event::Log(s));})?;Ok(Reply::Done("Shared root files restored.".into()))});}
                ui.separator();ui.label("A root recovery copy restores Android data to the time it was saved. Newer app data in that instance is replaced. Restore newer root operations first.");
                if ui.button("Show root recovery copies").clicked(){let state=self.root.clone();self.job(ctx,"Reading root recovery copies…",move |_|Ok(Reply::RootBackups(rooting::backups(&state)?)));}
                let mut chosen=None;for backup in &self.root_backups{ui.group(|ui|{ui.label(format!("{} · {} · {}",backup.instance,backup.created,backup.stage));if ui.add_enabled(backup.stage!="Restored"&&platform::is_admin(),egui::Button::new("Restore this root recovery copy")).clicked(){chosen=Some(backup.path.clone());}});}
                if let Some(path)=chosen{let state=self.root.clone();self.root_status=None;self.job(ctx,"Restoring the selected root recovery copy…",move |tx|{rooting::restore_backup(&path,&state,|s|{let _=tx.send(Event::Log(s));})?;Ok(Reply::Done("Root recovery copy restored.".into()))});}
                ui.collapsing("Technical paths",|ui|{ui.label(format!("System: {}",info.system_disk.display()));ui.label(format!("Android data: {}",info.data_disk.display()));});
            });
        }
    }
}
