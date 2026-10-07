#![windows_subsystem = "windows"]
mod ui;
use anyhow::{Context, Result};
use bluestacks_debloat::{
    adb, backup_cleanup, discovery,
    engine::{self, HostOptions, Performance},
    network, patch, platform, rooting, transaction,
};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "Native BlueStacks debloating with previews and verified backups. Run without arguments for the desktop app."
)]
struct Args {
    #[arg(long, global = true)]
    install: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "bluestacks.conf path, data root or Engine folder"
    )]
    conf: Option<PathBuf>,
    #[arg(long, global = true)]
    instance: Option<String>,
    #[command(subcommand)]
    command: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Native Magisk root, verification, unroot and shared-system restoration.
    Root {
        #[arg(value_enum)]
        action: RootAction,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        repair: bool,
    },
    /// List full root-operation recovery copies.
    RootBackups,
    /// Restore a saved root operation, including Android data at backup time.
    RootRestore { backup: PathBuf },
    /// Read installation and instance information; does not start/stop BlueStacks.
    Scan,
    /// Preview host changes. Add --apply to commit exactly this set.
    Host {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        maximum: bool,
        #[arg(long)]
        cloud: bool,
        #[arg(long)]
        remove_x: bool,
        #[arg(long)]
        remove_services: bool,
        #[arg(long)]
        quiet: bool,
        #[arg(long)]
        gpu: bool,
        #[arg(long)]
        high_fps: bool,
        #[arg(long)]
        hosts: bool,
        #[arg(long)]
        patch: bool,
        #[arg(long)]
        enable_adb: bool,
        #[arg(long, value_enum, default_value = "keep")]
        performance: Performance,
    },
    /// List reviewed Android package candidates from the selected running instance.
    Packages,
    /// Preview selected package/animation changes; add --apply to commit.
    Guest {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        recommended: bool,
        #[arg(long)]
        animations: bool,
        #[arg(long)]
        package: Vec<String>,
    },
    /// Optional Magisk module for Android hosts and launcher-only network filtering.
    RootNetwork {
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        hosts: bool,
        #[arg(long)]
        isolate_launcher: bool,
    },
    /// Sample TCP endpoints and domains from recent BlueStacks logs.
    Network,
    /// Inspect the player's disk-integrity patch without writing it.
    PatchInfo,
    /// Save a screenshot of the selected running Android instance.
    Screenshot { output: PathBuf },
    /// Start a specific instance.
    Launch,
    /// Request a normal close through the player's Windows UI.
    Close,
    /// List available operation backups.
    Backups,
    /// Fully verify recovery journals, files and checksums without restoring.
    VerifyBackups,
    /// Enforce the fixed newest-three retention rule, protecting recovery problems.
    CleanupBackups,
    /// Permanently delete one completed recovery point, including its cloud copies.
    DeleteBackup { backup: PathBuf },
    /// Restore one exact backup directory (Android backups require the instance running).
    Restore { backup: PathBuf },
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum RootAction {
    Inspect,
    Install,
    Verify,
    Unroot,
    FullUnroot,
}
fn json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
fn main() {
    let cli = std::env::args_os().len() > 1;
    if cli {
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    let result = if cli { run_cli() } else { ui::run() };
    if let Err(e) = result {
        if cli {
            eprintln!("Error: {e:#}");
        } else {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    platform::wide(format!("{e:#}")).as_ptr(),
                    platform::wide("BlueStacks Debloat").as_ptr(),
                    windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
        }
        std::process::exit(1);
    }
}
fn run_cli() -> Result<()> {
    let args = Args::parse();
    let root = platform::state_dir();
    if let Some(Action::RootBackups) = args.command {
        return json(&rooting::backups(&root)?);
    }
    if let Some(Action::RootRestore { ref backup }) = args.command {
        return rooting::restore_backup(backup, &root, |s| eprintln!("{s}"));
    }
    if let Some(Action::Backups) = args.command {
        return json(&backup_cleanup::list(&root)?);
    }
    if let Some(Action::VerifyBackups) = args.command {
        return json(&backup_cleanup::audit(&root)?);
    }
    if let Some(Action::CleanupBackups) = args.command {
        return json(&backup_cleanup::prune(&root, |s| eprintln!("{s}"))?);
    }
    if let Some(Action::DeleteBackup { ref backup }) = args.command {
        return backup_cleanup::delete(&root, backup);
    }
    if let Some(Action::Restore { ref backup }) = args.command {
        return transaction::restore(backup, &root, |s| eprintln!("{s}"));
    }
    let install = discovery::select(args.install.as_deref(), args.conf.as_deref())?;
    let snapshot = discovery::snapshot(install.clone())?;
    let selected = || {
        args.instance
            .as_deref()
            .or_else(|| {
                if snapshot.instances.len() == 1 {
                    Some(snapshot.instances[0].name.as_str())
                } else {
                    None
                }
            })
            .context("Use --instance NAME; the installation contains multiple instances")
    };
    match args.command {
        Some(Action::Root {
            action,
            apply,
            repair,
        }) => {
            let info = rooting::info(&install, selected()?)?;
            match action {
                RootAction::Inspect => json(&info)?,
                RootAction::Verify => json(&rooting::verify(&info, &mut |s| eprintln!("{s}"))?)?,
                RootAction::Install if apply => {
                    json(&rooting::install(info, repair, &root, |s| {
                        eprintln!("{s}")
                    })?)?
                }
                RootAction::Unroot if apply => rooting::unroot(info, &root, |s| eprintln!("{s}"))?,
                RootAction::FullUnroot if apply => {
                    rooting::full_unroot(install, &root, |s| eprintln!("{s}"))?
                }
                _ => {
                    json(&info)?;
                    println!(
                        "Preview only. Use --apply to run the selected root action. Rooting restarts BlueStacks and saves full disk recovery copies."
                    );
                }
            }
        }
        Some(Action::RootBackups | Action::RootRestore { .. }) => unreachable!(),
        Some(Action::Scan) => json(&snapshot)?,
        Some(Action::Host {
            apply,
            maximum,
            cloud,
            remove_x,
            remove_services,
            quiet,
            gpu,
            high_fps,
            hosts,
            patch,
            enable_adb,
            performance,
        }) => {
            let mut o = if maximum {
                HostOptions::maximum()
            } else {
                HostOptions::default()
            };
            o.cloud |= cloud;
            o.remove_x |= remove_x;
            o.remove_services |= remove_services;
            o.quiet |= quiet;
            o.gpu |= gpu;
            o.high_fps |= high_fps;
            o.hosts |= hosts;
            o.patch = patch;
            o.enable_adb |= enable_adb;
            o.performance = performance;
            let plan = engine::host_plan(&snapshot, selected()?, &o)?;
            json(&plan.summary())?;
            if apply && !plan.operations.is_empty() {
                transaction::apply(plan, &root, |s| eprintln!("{s}"))?;
            }
        }
        Some(Action::Packages) => json(&engine::guest_scan(&install, selected()?)?.1)?,
        Some(Action::Guest {
            apply,
            recommended,
            animations,
            mut package,
        }) => {
            if recommended {
                package.extend(
                    engine::guest_scan(&install, selected()?)?
                        .1
                        .into_iter()
                        .filter(|p| p.recommended)
                        .map(|p| p.name),
                );
            }
            package.sort();
            package.dedup();
            let plan = engine::guest_plan(&install, selected()?, &package, animations)?;
            json(&plan.summary())?;
            if apply && !plan.operations.is_empty() {
                transaction::apply(plan, &root, |s| eprintln!("{s}"))?;
            }
        }
        Some(Action::Network) => json(&network::report(&install)?)?,
        Some(Action::RootNetwork {
            apply,
            hosts,
            isolate_launcher,
        }) => {
            let plan = engine::root_plan(&install, selected()?, hosts, isolate_launcher)?;
            json(&plan.summary())?;
            if apply && !plan.operations.is_empty() {
                transaction::apply(plan, &root, |s| eprintln!("{s}"))?;
            }
        }
        Some(Action::PatchInfo) => json(&patch::inspect(&std::fs::read(install.player())?)?)?,
        Some(Action::Screenshot { output }) => {
            adb::Client::connect(&install, selected()?)?.screenshot(&output)?;
            println!("{}", output.display());
        }
        Some(Action::Launch) => discovery::launch(&install, selected()?)?,
        Some(Action::Close) => {
            let pids = snapshot
                .processes
                .iter()
                .filter(|p| {
                    p.name.eq_ignore_ascii_case("HD-Player.exe")
                        && p.instance.as_deref() == args.instance.as_deref()
                })
                .map(|p| p.pid)
                .collect::<Vec<_>>();
            platform::close_windows(&pids);
            println!("Close requested; complete any BlueStacks exit dialog before host changes.");
        }
        Some(
            Action::Backups
            | Action::Restore { .. }
            | Action::VerifyBackups
            | Action::CleanupBackups
            | Action::DeleteBackup { .. },
        ) => unreachable!(),
        None => ui::run()?,
    }
    Ok(())
}
