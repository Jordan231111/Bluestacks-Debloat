use super::*;
impl App {
    fn recommended(&mut self) {
        self.options = HostOptions::default();
        self.maximum_selected = false;
        self.root_hosts = false;
        self.root_isolate = false;
        self.animations = true;
        self.selected = self
            .packages
            .iter()
            .filter(|p| p.recommended && p.enabled <= 1)
            .map(|p| p.name.clone())
            .collect();
        self.clear_preview();
    }
    fn maximum(&mut self) {
        self.options = HostOptions::maximum();
        self.maximum_selected = true;
        self.animations = true;
        self.root_hosts = true;
        self.root_isolate = true;
        self.selected = self
            .packages
            .iter()
            .filter(|p| p.enabled <= 1)
            .map(|p| p.name.clone())
            .collect();
        self.clear_preview();
        self.status =
            "Maximum selected. CPU cores and RAM are preserved. Review each stage before applying."
                .into();
    }
    fn host_primary(&mut self, ui: &mut egui::Ui) {
        ui.strong("Debloat");
        ui.checkbox(&mut self.options.ads, "Disable BlueStacks ad controls");
        ui.checkbox(
            &mut self.options.telemetry,
            "Disable BlueStacks statistics reporting",
        );
        ui.checkbox(
            &mut self.options.smart_downloads,
            "Stop automatic game downloads",
        );
        ui.checkbox(
            &mut self.options.quiet,
            "Turn off this instance's notifications",
        );
        ui.checkbox(
            &mut self.options.cloud,
            "Disable cloud, rewards and AI integrations",
        );
        ui.checkbox(&mut self.options.remove_x, "Remove BlueStacks X / Store");
        ui.checkbox(
            &mut self.options.remove_services,
            "Remove the cloud Services companion",
        );
        ui.checkbox(
            &mut self.options.keep_features,
            "Keep these choices after restart (player patch)",
        );
        ui.label(RichText::new("Stops BlueStacks' online configuration refresh from resetting feature flags. Included in Maximum; reversible from Backups.").small().color(MUTED));
    }
    fn host_tuning(&mut self, ui: &mut egui::Ui) {
        ui.strong("Performance & tools");
        ui.checkbox(&mut self.options.gpu, "Prefer the high-performance GPU");
        ui.checkbox(
            &mut self.options.high_fps,
            "Enable up to 240 FPS and disable VSync",
        );
        ui.label(RichText::new("A higher limit can increase power use. Games and the display determine the frame rate you actually get.").small().color(MUTED));
        ui.checkbox(
            &mut self.options.enable_adb,
            "Enable local Android tools (ADB)",
        );
        ui.checkbox(
            &mut self.options.hosts,
            "Block BlueStacks ad endpoints on this PC",
        );
        ui.label(
            RichText::new("Only BlueStacks hostnames are included. Review them on Network.")
                .small()
                .color(MUTED),
        );
        ui.collapsing("Optional CPU / RAM changes (manual)",|ui|{
            ui.label("Maximum keeps these allocations unchanged. Change this setting only if you want a different resource budget.");
            egui::ComboBox::from_id_salt("performance").width(ui.available_width().min(300.0)).selected_text(match self.options.performance{Performance::Keep=>"Keep current CPU / RAM",Performance::Balanced=>"Balanced",Performance::Gaming=>"Gaming",Performance::LowMemory=>"Low memory"}).show_ui(ui,|ui|{for(value,label)in[(Performance::Keep,"Keep current CPU / RAM"),(Performance::Balanced,"Up to 4 cores / 4 GB"),(Performance::Gaming,"Up to 8 cores / 8 GB"),(Performance::LowMemory,"Up to 2 cores / 2 GB")]{ui.selectable_value(&mut self.options.performance,value,label);}});
        });
    }
    pub(super) fn host(&mut self, ui: &mut egui::Ui) {
        Self::heading(
            ui,
            "Debloat & tune",
            "Checkboxes select options. Use Review and Apply in the fixed bottom bar.",
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("Recommended").clicked() {
                self.recommended();
            }
            if ui.button("Maximum debloat").clicked() {
                self.maximum();
            }
        });
        if self.maximum_selected && self.options.performance == Performance::Keep {
            egui::Frame::group(ui.style()).inner_margin(12.0).show(ui,|ui|{
            ui.strong("Maximum · CPU and RAM kept as they are");ui.label("Selects reviewed cleanup, cloud removal, GPU / high-FPS options, Android options and BlueStacks-only network controls. Includes a reversible player patch to keep feature choices after restart. Rooting remains separate.");
            ui.label(RichText::new("Apply host changes using the bottom bar, then finish Android apps and Network. Root-only filtering needs Magisk.").color(MUTED));
        });
        }
        ui.add_space(10.0);
        let before = format!("{:?}", self.options);
        if ui.available_width() >= 730.0 {
            ui.columns(2, |columns| {
                self.host_primary(&mut columns[0]);
                self.host_tuning(&mut columns[1]);
            });
        } else {
            self.host_primary(ui);
            ui.add_space(12.0);
            self.host_tuning(ui);
        }
        ui.collapsing("Advanced player patch",|ui|{ui.checkbox(&mut self.options.patch,"Patch system-disk integrity checks");ui.label(RichText::new("The Root tab handles this automatically when rooting. It affects all instances and changes the player's signature. It is not a performance tweak.").small().color(MUTED));});
        if before != format!("{:?}", self.options) {
            self.clear_preview();
        }
        ui.add_space(12.0);
        self.preview_details(ui);
    }
}
