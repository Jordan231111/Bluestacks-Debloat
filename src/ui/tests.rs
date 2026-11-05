use super::*;

fn runtime_snapshot(root: &std::path::Path) -> Snapshot {
    Snapshot {
        installation: Installation {
            install_dir: root.join("program"),
            data_dir: root.join("data"),
            version: "fixture".into(),
            source: "test".into(),
        },
        instances: ["Pie64", "Tiramisu64"]
            .map(|name| discovery::Instance {
                name: name.into(),
                display_name: name.into(),
                adb_port: None,
                cpus: 4,
                ram_mb: 4096,
                fps: 240,
            })
            .into(),
        processes: vec![discovery::Process {
            pid: 42,
            name: "HD-Player.exe".into(),
            path: None,
            instance: Some("Tiramisu64".into()),
            memory_mb: 100,
            start_time: 1,
        }],
        cpu_count: 16,
        ram_mb: 32768,
        admin: true,
    }
}

#[test]
fn runtime_refresh_updates_status_without_losing_selections_or_results() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::blank(root.path().into());
    let snapshot = runtime_snapshot(root.path());
    app.snapshot = Some(snapshot.clone());
    app.instances.insert("Tiramisu64".into());
    app.selected.insert("com.android.traceur".into());
    app.batch_report = Some(batch::BatchReport { instances: vec![] });
    app.status = "Apply completed".into();
    let mut stopped = snapshot;
    stopped.processes.clear();
    app.tx.send(Event::Refreshed(stopped)).unwrap();
    app.receive();
    assert!(app.snapshot.as_ref().unwrap().processes.is_empty());
    assert!(app.instances.contains("Tiramisu64"));
    assert!(app.selected.contains("com.android.traceur"));
    assert!(app.batch_report.is_some());
    assert_eq!(app.status, "Apply completed");
}

#[test]
fn deleted_saved_instance_falls_back_to_selection_then_running_instance() {
    let root = tempfile::tempdir().unwrap();
    for (saved, selected, expected) in [
        ("Tiramisu64_deleted", Some("Pie64"), "Pie64"),
        ("Tiramisu64_deleted", None, "Tiramisu64"),
        ("Pie64", Some("Tiramisu64"), "Pie64"),
    ] {
        let mut app = App::blank(root.path().into());
        app.instance = saved.into();
        if let Some(selected) = selected {
            app.instances.insert(selected.into());
        }
        app.selection_initialized = true;
        app.tx
            .send(Event::Finished(Box::new(Ok(Reply::Scanned(
                runtime_snapshot(root.path()),
            )))))
            .unwrap();
        app.receive();
        assert_eq!(app.instance, expected);
    }
}

#[test]
fn all_tabs_survive_repeated_layouts_at_supported_sizes_and_text_scales() {
    let root = tempfile::tempdir().unwrap();
    let mut app = App::blank(root.path().to_owned());
    let install = Installation {
        install_dir: root.path().join("program"),
        data_dir: root.path().join("data"),
        version: "layout fixture".into(),
        source: "test".into(),
    };
    let instances = (0..40)
        .map(|i| discovery::Instance {
            name: format!("Rvc64_{i}"),
            display_name: format!(
                "Instance {i} with a long display name for wrapping and scrolling"
            ),
            adb_port: None,
            cpus: 2,
            ram_mb: 2048,
            fps: 60,
        })
        .collect::<Vec<_>>();
    app.instance = instances[0].name.clone();
    app.instances = instances.iter().map(|i| i.name.clone()).collect();
    app.selection_initialized = true;
    app.layout_initialized = true;
    app.snapshot = Some(Snapshot {
        installation: install.clone(),
        instances,
        processes: vec![],
        cpu_count: 16,
        ram_mb: 32768,
        admin: true,
    });
    app.root_info = Some(rooting::RootInfo {
        installation: install,
        instance: app.instance.clone(),
        system_disk: root.path().join("Root.vhd"),
        data_disk: root.path().join("Data.vhdx"),
        machine_file: root.path().join("fixture.bstk"),
        master_file: None,
        shared_instances: vec![app.instance.clone()],
        backup_bytes: 8 * 1024 * 1024 * 1024,
        supported: true,
    });
    app.packages = vec![Package {
        name: "com.android.egg".into(),
        enabled: 0,
        reason: "Optional Android Easter egg".into(),
        recommended: false,
    }];
    let context = egui::Context::default();
    context.set_theme(egui::Theme::Dark);
    let mut frames = 0;
    for physical in [
        egui::vec2(600.0, 700.0),
        egui::vec2(800.0, 620.0),
        egui::vec2(1120.0, 820.0),
        egui::vec2(1920.0, 1080.0),
    ] {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            context.set_zoom_factor(scale);
            for &(tab, _) in responsive::TABS {
                app.tab = tab;
                for state in 0..3 {
                    app.busy = state == 2;
                    app.error = (state == 1).then(|| "A deliberately long diagnostic error to check wrapping and recovery controls at every supported window size".into());
                    for _ in 0..2 {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                physical / scale,
                            )),
                            ..Default::default()
                        };
                        let output = context.run_ui(input, |ui| app.render(ui));
                        assert!(
                            output.shapes.iter().all(|s| s.clip_rect.is_finite()),
                            "Invalid clipping on {tab:?}, {physical:?}, scale {scale}"
                        );
                        output.drop_without_applying_deltas();
                        frames += 1;
                    }
                    assert_eq!(
                        app.busy,
                        state == 2,
                        "Rendering must not start background work"
                    );
                }
            }
        }
    }
    assert_eq!(frames, 840);
    assert!(!root.path().join("ui-preferences.json").exists());
}
