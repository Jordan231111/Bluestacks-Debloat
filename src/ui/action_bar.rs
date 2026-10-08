use super::*;

#[derive(Clone, Copy)]
pub(super) enum ReviewAction {
    Review,
    Apply,
}

pub(super) struct AppliedChanges {
    pub report: transaction::ApplyReport,
    pub time: String,
}

impl App {
    pub(super) fn changes(count: usize) -> String {
        format!("{count} {}", if count == 1 { "change" } else { "changes" })
    }
    pub(super) fn has_review_actions(&self) -> bool {
        matches!(
            self.tab,
            Tab::Host | Tab::Cloud | Tab::Android | Tab::Network
        )
    }

    pub(super) fn change_count(plan: &Plan) -> usize {
        plan.operations
            .iter()
            .map(|operation| match &operation.target {
                Target::Config { edits, .. } => edits.len(),
                _ => 1,
            })
            .sum()
    }

    pub(super) fn clear_preview(&mut self) {
        self.plan = None;
        self.applied = None;
        self.scroll_to_preview = false;
    }

    pub(super) fn action_bar(&mut self, ui: &mut egui::Ui) -> Option<ReviewAction> {
        let count = self.plan.as_ref().map(Self::change_count);
        let can_apply = !self.busy
            && self.error.is_none()
            && self.plan.as_ref().is_some_and(|p| {
                !p.operations.is_empty() && (p.guest.is_some() || platform::is_admin())
            });
        let mut action = None;
        let (title, hint, color) = if self.busy {
            (
                self.status.clone(),
                "Please wait for verification to finish.",
                MUTED,
            )
        } else if self.error.is_some() {
            (
                "Action needs attention".into(),
                "Check the error details. Review again before applying.",
                Color32::from_rgb(255, 169, 157),
            )
        } else if let Some(count) = count {
            if count == 0 {
                let unavailable = self
                    .plan
                    .as_ref()
                    .is_some_and(|plan| !plan.issues.is_empty());
                (
                    if unavailable {
                        "Selected options need attention"
                    } else {
                        "No changes needed"
                    }
                    .into(),
                    if unavailable {
                        "Review the unavailable options below. Nothing has been applied."
                    } else {
                        "Selected settings are already configured."
                    },
                    if unavailable {
                        Color32::from_rgb(244, 204, 128)
                    } else {
                        ACCENT
                    },
                )
            } else {
                let hint = if self.plan.as_ref().is_some_and(|p| p.guest.is_none()) {
                    if platform::is_admin() {
                        "Apply automatically closes BlueStacks and its companions before making these changes."
                    } else {
                        "Review only. Restart as administrator to apply host changes."
                    }
                } else {
                    "Review only. Click Apply to change the selected Android instance."
                };
                (
                    format!(
                        "{} ready — not applied yet{}",
                        Self::changes(count),
                        self.plan
                            .as_ref()
                            .filter(|plan| !plan.issues.is_empty())
                            .map(|plan| format!(
                                "; {} unavailable",
                                plan.issues.iter().map(|issue| issue.changes).sum::<usize>()
                            ))
                            .unwrap_or_default()
                    ),
                    hint,
                    Color32::from_rgb(244, 204, 128),
                )
            }
        } else if let Some(applied) = &self.applied {
            (
                format!(
                    "{} at {}",
                    applied.report.summary().trim_end_matches('.'),
                    applied.time
                ),
                if applied.report.has_issues() {
                    "Successful changes remain applied. Open Activity for errors, then Review to retry the remaining options."
                } else {
                    "Recovery saved. Use Backups & restore to undo this operation."
                },
                if applied.report.has_issues() {
                    Color32::from_rgb(244, 204, 128)
                } else {
                    ACCENT
                },
            )
        } else {
            (
                "Review your selected options".into(),
                "Checkboxes select options. Review checks what still needs changing.",
                MUTED,
            )
        };
        let compact = ui.available_width() < 480.0;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.busy && self.snapshot.is_some(),
                    egui::Button::new(if compact { "1. Review" } else { "1. Review changes" })
                        .min_size(egui::vec2(0.0, 40.0)),
                )
                .on_hover_text("Checks the current settings and shows what would change. Does not apply changes.")
                .clicked()
            {
                action = Some(ReviewAction::Review);
            }
            let label = if compact {
                "2. Apply".into()
            } else if let Some(count) = count.filter(|count| *count > 0) {
                format!("2. Apply {}", Self::changes(count))
            } else {
                "2. Apply changes".into()
            };
            if ui
                .add_enabled(
                    can_apply,
                    egui::Button::new(RichText::new(label).strong())
                        .fill(Color32::from_rgb(35, 100, 91))
                        .min_size(egui::vec2(0.0, 40.0)),
                )
                .on_disabled_hover_text("Review your selections first. Host changes also require administrator access.")
                .clicked()
            {
                action = Some(ReviewAction::Apply);
            }
            if let Some(applied) = &self.applied
                && let Some(backup) = &applied.report.backup
                && ui.small_button("Open recovery copy").clicked()
                && let Err(error) = platform::open_folder(backup)
            {
                self.error = Some(format!("{error:#}"));
            }
        });
        if self.busy {
            ui.add(egui::Label::new(RichText::new(title).strong().color(color)).truncate());
        } else {
            ui.label(RichText::new(title).strong().color(color));
        }
        // At very short logical heights, keep both actions visible above all else.
        if ui.ctx().content_rect().height() >= 340.0 {
            ui.label(RichText::new(hint).small().color(MUTED));
        }
        action
    }

    pub(super) fn run_review_action(&mut self, action: ReviewAction, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        match action {
            ReviewAction::Review => {
                self.clear_preview();
                match self.tab {
                    Tab::Host | Tab::Cloud => self.preview_host(ctx, self.tab == Tab::Cloud),
                    Tab::Android => {
                        if let Some(install) = self.install() {
                            let name = self.instance.clone();
                            let selected = self.selected.iter().cloned().collect::<Vec<_>>();
                            let animations = self.animations;
                            self.job(ctx, "Reviewing Android changes…", move |_| {
                                Ok(Reply::Preview(engine::guest_plan(
                                    &install, &name, &selected, animations,
                                )?))
                            });
                        }
                    }
                    Tab::Network => {
                        if let Some(install) = self.install() {
                            let name = self.instance.clone();
                            let hosts = self.root_hosts;
                            let isolate = self.root_isolate;
                            self.job(ctx, "Reviewing root network changes…", move |_| {
                                Ok(Reply::Preview(engine::root_plan(
                                    &install, &name, hosts, isolate,
                                )?))
                            });
                        }
                    }
                    _ => {}
                }
            }
            ReviewAction::Apply => {
                if self.error.is_some()
                    || self.plan.as_ref().is_none_or(|p| p.operations.is_empty())
                {
                    return;
                }
                if let Some(plan) = self.plan.take() {
                    let root = self.root.clone();
                    self.applied = None;
                    self.job(ctx, "Applying and verifying changes…", move |tx| {
                        let report = transaction::apply(plan, &root, |s| {
                            let _ = tx.send(Event::Log(s));
                        })?;
                        Ok(Reply::Applied(report))
                    });
                }
            }
        }
    }
}
