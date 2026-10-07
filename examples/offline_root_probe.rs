//! Test the native filesystem edit against an explicitly supplied expendable VHD.
fn main() -> anyhow::Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 2,
        "Usage: offline_root_probe COPIED_VHD NEW_WORKSPACE"
    );
    let assets = bluestacks_debloat::root_assets::extract(std::path::Path::new(&args[1]))?;
    bluestacks_debloat::root_files::edit_system(
        std::path::Path::new(&args[0]),
        &assets,
        &bluestacks_debloat::root_files::prep_files(&assets),
        &["/android/system/xbin/daemonsu"],
        &mut |s| eprintln!("{s}"),
    )
}
