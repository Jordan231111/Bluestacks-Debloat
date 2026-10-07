//! Windows virtual-disk attachment plus bounded, sector-aligned ext4 staging.
//! A physical drive is obtained only from the handle of the requested VHD/VHDX.
use crate::platform;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    Storage::Vhd::*,
    System::{
        IO::DeviceIoControl,
        Ioctl::IOCTL_DISK_GET_LENGTH_INFO,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Format {
    Vhd,
    Vhdx,
}
pub fn format(path: &Path) -> Result<Format> {
    let mut f = File::open(path)?;
    ensure!(f.metadata()?.len() >= 512, "Virtual disk is truncated");
    let mut b = [0; 512];
    f.read_exact(&mut b)?;
    if &b[..8] == b"vhdxfile" {
        return Ok(Format::Vhdx);
    }
    f.seek(SeekFrom::End(-512))?;
    f.read_exact(&mut b)?;
    ensure!(
        &b[..8] == b"conectix",
        "Expected a VHD or VHDX by its content, not merely its filename"
    );
    Ok(Format::Vhd)
}
fn privilege() -> Result<()> {
    let mut token = std::ptr::null_mut();
    ensure!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        } != 0,
        "Open process token: {}",
        std::io::Error::last_os_error()
    );
    let result = (|| {
        let mut luid = LUID::default();
        ensure!(
            unsafe {
                LookupPrivilegeValueW(
                    std::ptr::null(),
                    platform::wide("SeManageVolumePrivilege").as_ptr(),
                    &mut luid,
                )
            } != 0,
            "Find volume-management privilege"
        );
        let p = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        ensure!(
            unsafe {
                AdjustTokenPrivileges(token, 0, &p, 0, std::ptr::null_mut(), std::ptr::null_mut())
            } != 0
                && unsafe { GetLastError() } != ERROR_NOT_ALL_ASSIGNED,
            "Administrator volume-management privilege is required"
        );
        Ok(())
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Region {
    pub start: u64,
    pub length: u64,
}
pub struct Disk {
    handle: HANDLE,
    attached: bool,
    file: Option<File>,
    pub device: String,
    pub length: u64,
    pub read_only: bool,
}
impl Disk {
    pub fn attach(path: &Path, read_only: bool) -> Result<Self> {
        privilege()?;
        let kind = format(path)?;
        let storage = VIRTUAL_STORAGE_TYPE {
            DeviceId: match kind {
                Format::Vhd => VIRTUAL_STORAGE_TYPE_DEVICE_VHD,
                Format::Vhdx => VIRTUAL_STORAGE_TYPE_DEVICE_VHDX,
            },
            VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
        };
        let parameters = OPEN_VIRTUAL_DISK_PARAMETERS {
            Version: OPEN_VIRTUAL_DISK_VERSION_1,
            Anonymous: OPEN_VIRTUAL_DISK_PARAMETERS_0 {
                Version1: OPEN_VIRTUAL_DISK_PARAMETERS_0_0 { RWDepth: 1 },
            },
        };
        let access = VIRTUAL_DISK_ACCESS_GET_INFO
            | VIRTUAL_DISK_ACCESS_DETACH
            | if read_only {
                VIRTUAL_DISK_ACCESS_ATTACH_RO
            } else {
                VIRTUAL_DISK_ACCESS_ATTACH_RW
            };
        let mut handle = std::ptr::null_mut();
        let status = unsafe {
            OpenVirtualDisk(
                &storage,
                platform::wide(path).as_ptr(),
                access,
                OPEN_VIRTUAL_DISK_FLAG_NONE,
                &parameters,
                &mut handle,
            )
        };
        ensure!(
            status == 0,
            "Open virtual disk {}: {} (Win32 {status})",
            path.display(),
            std::io::Error::from_raw_os_error(status as i32)
        );
        let mut disk = Self {
            handle,
            attached: false,
            file: None,
            device: String::new(),
            length: 0,
            read_only,
        };
        ensure!(
            disk.physical_path().is_err(),
            "Virtual disk is already attached; detach it in Disk Management before rooting"
        );
        let attach = ATTACH_VIRTUAL_DISK_PARAMETERS {
            Version: ATTACH_VIRTUAL_DISK_VERSION_1,
            ..Default::default()
        };
        let flags = ATTACH_VIRTUAL_DISK_FLAG_NO_DRIVE_LETTER
            | if read_only {
                ATTACH_VIRTUAL_DISK_FLAG_READ_ONLY
            } else {
                0
            };
        let status = unsafe {
            AttachVirtualDisk(
                handle,
                std::ptr::null_mut(),
                flags,
                0,
                &attach,
                std::ptr::null(),
            )
        };
        ensure!(
            status == 0,
            "Attach {}: {} (Win32 {status}); close BlueStacks and use an uncompressed, unencrypted local volume",
            path.display(),
            std::io::Error::from_raw_os_error(status as i32)
        );
        disk.attached = true;
        let start = Instant::now();
        loop {
            if let Ok(p) = disk.physical_path() {
                disk.device = p;
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(10),
                "Windows did not expose the attached virtual disk"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        ensure!(
            regex::Regex::new(r"^\\\\\.\\PhysicalDrive[0-9]+$")?.is_match(&disk.device),
            "Unexpected virtual-disk device path"
        );
        let file = OpenOptions::new()
            .read(true)
            .write(!read_only)
            .share_mode(3)
            .open(&disk.device)
            .with_context(|| format!("Open attached virtual disk {}", disk.device))?;
        let mut length = 0u64;
        let mut returned = 0;
        ensure!(
            unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    IOCTL_DISK_GET_LENGTH_INFO,
                    std::ptr::null(),
                    0,
                    (&mut length as *mut u64).cast(),
                    8,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            } != 0
                && returned == 8,
            "Query virtual disk length: {}",
            std::io::Error::last_os_error()
        );
        ensure!(
            length >= 4096 && length.is_multiple_of(512),
            "Invalid virtual disk length"
        );
        disk.file = Some(file);
        disk.length = length;
        Ok(disk)
    }
    fn physical_path(&self) -> Result<String> {
        let mut buf = [0u16; 512];
        let mut size = (buf.len() * 2) as u32;
        let code = unsafe { GetVirtualDiskPhysicalPath(self.handle, &mut size, buf.as_mut_ptr()) };
        ensure!(code == 0, "Virtual disk has no attached device ({code})");
        let end = buf
            .iter()
            .position(|b| *b == 0)
            .context("Malformed device path")?;
        Ok(String::from_utf16(&buf[..end])?)
    }
    pub fn read_at(&mut self, start: u64, size: usize) -> Result<Vec<u8>> {
        ensure!(
            start.is_multiple_of(512)
                && size.is_multiple_of(512)
                && start
                    .checked_add(size as u64)
                    .is_some_and(|e| e <= self.length),
            "Read outside virtual disk or not sector-aligned"
        );
        let f = self.file.as_mut().context("Disk is detached")?;
        f.seek(SeekFrom::Start(start))?;
        let mut b = vec![0; size];
        f.read_exact(&mut b)?;
        Ok(b)
    }
    pub fn ext4(&mut self) -> Result<Region> {
        let head = self.read_at(0, 4096)?;
        let mut candidates = Vec::new();
        if head.get(0x438..0x43a) == Some(&[0x53, 0xef]) {
            candidates.push(Region {
                start: 0,
                length: self.length,
            });
        }
        if head[510..512] == [0x55, 0xaa] {
            if head[512..520] == *b"EFI PART" {
                let header = &head[512..1024];
                let size = u32::from_le_bytes(header[12..16].try_into()?) as usize;
                ensure!((92..=512).contains(&size), "Invalid GPT header size");
                let expected = u32::from_le_bytes(header[16..20].try_into()?);
                let mut checked = header[..size].to_vec();
                checked[16..20].fill(0);
                ensure!(
                    crc32fast::hash(&checked) == expected,
                    "GPT header checksum mismatch"
                );
                let sector = u64::from_le_bytes(header[72..80].try_into()?);
                let count = u32::from_le_bytes(header[80..84].try_into()?) as usize;
                let entry = u32::from_le_bytes(header[84..88].try_into()?) as usize;
                ensure!(
                    (1..=4096).contains(&count)
                        && (128..=1024).contains(&entry)
                        && entry.is_multiple_of(8),
                    "Unsupported GPT partition table"
                );
                let n = count * entry;
                let table = self.read_at(
                    sector.checked_mul(512).context("GPT sector overflow")?,
                    n.div_ceil(512) * 512,
                )?;
                ensure!(
                    crc32fast::hash(&table[..n]) == u32::from_le_bytes(header[88..92].try_into()?),
                    "GPT partition-table checksum mismatch"
                );
                for e in table[..n].chunks_exact(entry) {
                    if e[..16].iter().all(|b| *b == 0) {
                        continue;
                    }
                    let start = u64::from_le_bytes(e[32..40].try_into()?);
                    let end = u64::from_le_bytes(e[40..48].try_into()?);
                    ensure!(end >= start, "Invalid GPT partition range");
                    candidates.push(sectors(start, end - start + 1, self.length)?);
                }
            } else {
                candidates.extend(mbr_regions(&head[..512], self.length)?);
            }
        }
        let mut found = Vec::new();
        for r in candidates {
            if r.length < 4096 {
                continue;
            }
            let b = self.read_at(r.start, 4096)?;
            if b[0x438..0x43a] == [0x53, 0xef] {
                found.push(r);
            }
        }
        found.sort_by_key(|r| r.start);
        found.dedup();
        ensure!(
            found.len() == 1,
            "Expected exactly one ext4 system partition; found {}",
            found.len()
        );
        Ok(found[0])
    }
    pub fn carve(
        &mut self,
        region: Region,
        path: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        validate_region(region, self.length)?;
        let input = self.file.as_mut().context("Disk detached")?;
        input.seek(SeekFrom::Start(region.start))?;
        let mut out = OpenOptions::new().create_new(true).write(true).open(path)?;
        copy_exact(input, &mut out, region.length, &mut progress)?;
        out.sync_all()?;
        ensure!(
            out.metadata()?.len() == region.length,
            "Incomplete partition copy"
        );
        Ok(())
    }
    pub fn write_back(
        &mut self,
        region: Region,
        path: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<()> {
        ensure!(
            !self.read_only,
            "Cannot write through a read-only virtual-disk handle"
        );
        validate_region(region, self.length)?;
        let mut input = File::open(path)?;
        ensure!(
            input.metadata()?.len() == region.length,
            "Edited partition changed size; refusing disk write"
        );
        let output = self.file.as_mut().context("Disk detached")?;
        let mut staged = vec![0; 4 * 1024 * 1024];
        let mut existing = vec![0; staged.len()];
        let mut done = 0;
        while done < region.length {
            let n = (region.length - done).min(staged.len() as u64) as usize;
            input.read_exact(&mut staged[..n])?;
            output.seek(SeekFrom::Start(region.start + done))?;
            output.read_exact(&mut existing[..n])?;
            if staged[..n] != existing[..n] {
                output.seek(SeekFrom::Start(region.start + done))?;
                output.write_all(&staged[..n])?;
            }
            done += n as u64;
            progress(done, region.length);
        }
        output.sync_all()?;
        Ok(())
    }
    pub fn detach(mut self) -> Result<()> {
        self.file.take();
        let code = unsafe { DetachVirtualDisk(self.handle, DETACH_VIRTUAL_DISK_FLAG_NONE, 0) };
        ensure!(code == 0, "Windows could not detach virtual disk ({code})");
        self.attached = false;
        Ok(())
    }
}
impl Drop for Disk {
    fn drop(&mut self) {
        self.file.take();
        unsafe {
            if self.attached {
                DetachVirtualDisk(self.handle, DETACH_VIRTUAL_DISK_FLAG_NONE, 0);
            }
            CloseHandle(self.handle);
        }
    }
}
fn sectors(start: u64, count: u64, disk: u64) -> Result<Region> {
    let r = Region {
        start: start.checked_mul(512).context("Partition start overflow")?,
        length: count.checked_mul(512).context("Partition size overflow")?,
    };
    validate_region(r, disk)?;
    Ok(r)
}
fn validate_region(r: Region, disk: u64) -> Result<()> {
    ensure!(
        r.length > 0
            && r.start.is_multiple_of(512)
            && r.length.is_multiple_of(512)
            && r.start.checked_add(r.length).is_some_and(|e| e <= disk),
        "Partition exceeds the virtual disk"
    );
    Ok(())
}
fn mbr_regions(head: &[u8], disk: u64) -> Result<Vec<Region>> {
    ensure!(head.len() == 512, "Invalid MBR length");
    let mut result = Vec::new();
    for e in head[446..510].as_chunks::<16>().0 {
        if e[4] == 0 {
            continue;
        }
        ensure!(
            !matches!(e[4], 0x05 | 0x0f | 0x85 | 0xee),
            "Extended or protective MBR without a valid GPT is unsupported"
        );
        result.push(sectors(
            u32::from_le_bytes(e[8..12].try_into()?) as u64,
            u32::from_le_bytes(e[12..16].try_into()?) as u64,
            disk,
        )?);
    }
    Ok(result)
}
pub fn copy_exact(
    input: &mut impl Read,
    output: &mut impl Write,
    length: u64,
    progress: &mut impl FnMut(u64, u64),
) -> Result<()> {
    let mut buffer = vec![0; 4 * 1024 * 1024];
    let mut done = 0;
    while done < length {
        let n = (length - done).min(buffer.len() as u64) as usize;
        input
            .read_exact(&mut buffer[..n])
            .context("Source ended before the expected disk region was copied")?;
        output.write_all(&buffer[..n])?;
        done += n as u64;
        progress(done, length);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_out_of_bounds_and_overflowing_partitions() {
        assert!(sectors(u64::MAX, 1, u64::MAX).is_err());
        assert!(sectors(2, 9, 4096).is_err());
        assert!(sectors(1, 0, 4096).is_err());
    }
    #[test]
    fn rejects_truncated_disk_copies() {
        let mut source = Cursor::new(vec![1; 511]);
        let mut target = Vec::new();
        assert!(copy_exact(&mut source, &mut target, 512, &mut |_, _| {}).is_err());
        assert!(target.is_empty());
    }
    #[test]
    fn parses_primary_mbr_without_accepting_extended_partitions() {
        let mut b = [0u8; 512];
        b[450] = 0x83;
        b[454..458].copy_from_slice(&8u32.to_le_bytes());
        b[458..462].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(
            mbr_regions(&b, 8192).unwrap(),
            vec![Region {
                start: 4096,
                length: 4096
            }]
        );
        b[450] = 0x0f;
        assert!(mbr_regions(&b, 8192).is_err());
    }
    use std::io::Cursor;
}
