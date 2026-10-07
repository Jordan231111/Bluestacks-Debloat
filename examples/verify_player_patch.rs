//! Offline verification helper: never writes either executable.
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "Usage: verify_player_patch STOCK_PLAYER CURRENT_PLAYER"
    );
    let original = std::fs::read(&args[0])?;
    let current = std::fs::read(&args[1])?;
    let (expected, report) = bluestacks_debloat::patch::patched(&original)?;
    anyhow::ensure!(
        expected == current,
        "Rust patch output differs from the installed binary"
    );
    println!(
        "Patch equivalence verified: {} new site(s), {} already patched",
        report.offsets.len(),
        report.already_patched
    );
    Ok(())
}
