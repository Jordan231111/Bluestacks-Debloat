//! Read-only virtual-disk validation helper.
fn main() -> anyhow::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Usage: inspect_vhd IMAGE"))?;
    let mut disk =
        bluestacks_debloat::virtual_disk::Disk::attach(std::path::Path::new(&path), true)?;
    let region = disk.ext4()?;
    println!("{}: {} bytes; ext4 {:?}", disk.device, disk.length, region);
    disk.detach()?;
    Ok(())
}
