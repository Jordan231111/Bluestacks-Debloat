use super::*;

impl App {
    pub(super) fn backups(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Self::heading(
            ui,
            "Backups & restore",
            "Automatically keeps the newest 3 verified recovery points across debloat and root.",
        );
        ui.label("Older recovery points are permanently removed. Unfinished or damaged recoveries stay visible for inspection.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Check backup integrity").clicked() {
                let root = self.root.clone();
                self.job(ctx, "Checking recovery files and checksums…", move |_| {
                    Ok(Reply::Backups(backup_cleanup::audit(&root)?))
                });
            }
            if ui.button("Open backup folders").clicked()
                && let Err(error) = platform::open_folder(&self.root)
            {
                self.error = Some(format!("{error:#}"));
            }
        });
        ui.label(RichText::new("Close BlueStacks for host restores. Start the original instance for Android restores. Root recovery returns Android data to its saved point in time.").color(MUTED));
        let mut restore = None;
        let mut delete = None;
        for backup in &self.backups {
            egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
                ui.strong(&backup.title);
                ui.label(RichText::new(format!("{} · {:?} · {}", backup.created, backup.kind, backup.status)).small().color(MUTED));
                if let Some(problem) = &backup.problem {
                    ui.label(RichText::new("Recovery needs attention").color(Color32::from_rgb(255, 169, 157)));
                    ui.label(problem);
                } else if backup.verified {
                    ui.label(RichText::new("Recovery file checksums verified").color(ACCENT));
                } else {
                    ui.label(RichText::new("Files present. Use Check backup integrity to recheck their contents.").small().color(MUTED));
                }
                ui.horizontal_wrapped(|ui| {
                    let restorable = backup.problem.is_none() && !matches!(backup.status.as_str(), "restored" | "Restored" | "Deleting");
                    if ui.add_enabled(restorable, egui::Button::new("Restore backup")).clicked() { restore = Some((backup.path.clone(), backup.kind)); }
                    if ui.add_enabled(backup.can_delete, egui::Button::new("Delete backup")).clicked() { self.delete_backup = Some(backup.path.clone()); }
                    if ui.small_button("Open folder").clicked()
                        && let Err(error) = platform::open_folder(&backup.path)
                    { self.error = Some(format!("{error:#}")); }
                });
                if !backup.can_delete { ui.label(RichText::new("Protected: resolve or inspect this recovery before deletion.").small().color(MUTED)); }
                if self.delete_backup.as_ref() == Some(&backup.path) {
                    ui.label("Permanently delete this recovery point and its stored copies? You will lose the ability to undo this operation.");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Delete permanently").clicked() { delete = Some(backup.path.clone()); }
                        if ui.button("Cancel").clicked() { self.delete_backup = None; }
                    });
                }
            });
        }
        if let Some((path, kind)) = restore {
            self.root_status = None;
            let root = self.root.clone();
            self.job(
                ctx,
                "Checking the full backup before restoring…",
                move |tx| {
                    let log = |s| {
                        let _ = tx.send(Event::Log(s));
                    };
                    match kind {
                        backup_cleanup::Kind::Debloat => transaction::restore(&path, &root, log)?,
                        backup_cleanup::Kind::Root => rooting::restore_backup(&path, &root, log)?,
                    }
                    Ok(Reply::Done("Restoration verified.".into()))
                },
            );
        } else if let Some(path) = delete {
            self.delete_backup = None;
            let root = self.root.clone();
            self.job(ctx, "Deleting the selected recovery point…", move |tx| {
                backup_cleanup::delete(&root, &path)?;
                let _ = tx.send(Event::Log("Selected backup deleted.".into()));
                Ok(Reply::Backups(backup_cleanup::audit(&root)?))
            });
        }
        if self.backups.is_empty() {
            ui.label("No recovery points have been created yet.");
        }
    }
}
