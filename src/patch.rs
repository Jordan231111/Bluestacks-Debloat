//! Conservative port of BluestacksRoot/tools/bsr_engine.ps1's primary-anchor
//! detector. No broad fallback, forced match, or hard-coded offset is accepted.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchReport {
    pub offsets: Vec<usize>,
    pub already_patched: usize,
    pub sha256: String,
}
#[derive(Debug)]
struct Section {
    raw: usize,
    size: usize,
    rva: u32,
    text: bool,
}
fn u16at(b: &[u8], n: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(n..n + 2).context("Truncated PE")?.try_into()?,
    ))
}
fn u32at(b: &[u8], n: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(n..n + 4).context("Truncated PE")?.try_into()?,
    ))
}
pub fn inspect(b: &[u8]) -> Result<PatchReport> {
    ensure!(
        b.len() >= 0x100 && b.starts_with(b"MZ"),
        "Not a PE executable"
    );
    let pe = u32at(b, 0x3c)? as usize;
    ensure!(b.get(pe..pe + 4) == Some(b"PE\0\0"), "Invalid PE header");
    ensure!(
        u16at(b, pe + 4)? == 0x8664,
        "Only x64 HD-Player builds are supported"
    );
    let n = u16at(b, pe + 6)? as usize;
    let opt = u16at(b, pe + 20)? as usize;
    ensure!(
        n > 0 && n < 97 && opt >= 32 && u16at(b, pe + 24)? == 0x20b,
        "Invalid PE optional header"
    );
    let table = pe + 24 + opt;
    ensure!(table + n * 40 <= b.len(), "Truncated PE section table");
    let mut sections = Vec::new();
    for i in 0..n {
        let s = table + i * 40;
        let raw = u32at(b, s + 20)? as usize;
        let size = u32at(b, s + 16)? as usize;
        ensure!(
            size == 0
                || raw >= table + n * 40 && raw.checked_add(size).is_some_and(|e| e <= b.len()),
            "Invalid section bounds"
        );
        sections.push(Section {
            raw,
            size,
            rva: u32at(b, s + 12)?,
            text: &b[s..s + 8] == b".text\0\0\0",
        });
    }
    let text: Vec<_> = sections.iter().filter(|s| s.text).collect();
    ensure!(text.len() == 1, "Expected one .text section");
    let text = text[0];
    let mut anchors = BTreeSet::new();
    for needle in [
        b"Verified the disk integrity!\0".as_slice(),
        b"Failed to verify the disk integrity!\0".as_slice(),
    ] {
        for (offset, _) in b
            .windows(needle.len())
            .enumerate()
            .filter(|(_, w)| *w == needle)
        {
            if let Some(s) = sections
                .iter()
                .find(|s| offset >= s.raw && offset < s.raw + s.size)
            {
                anchors.insert(s.rva as i64 + (offset - s.raw) as i64);
            }
        }
    }
    ensure!(
        !anchors.is_empty(),
        "No primary integrity anchors; this build cannot be patched safely"
    );
    let mut offsets = Vec::new();
    let mut already_patched = 0;
    for t in text.raw + 5..(text.raw + text.size).saturating_sub(3) {
        if b[t - 5] != 0xe8
            || b[t..t + 2] != [0x84, 0xc0]
            || !(b[t + 2] == 0x74 || b[t + 2..t + 4] == [0x90, 0x90])
        {
            continue;
        }
        // Validate CALL target and short conditional destination inside executable code.
        let call = text.rva as i64
            + (t - text.raw) as i64
            + i32::from_le_bytes(b[t - 4..t].try_into()?) as i64;
        if call < text.rva as i64 || call >= text.rva as i64 + text.size as i64 {
            continue;
        }
        if b[t + 2] == 0x74 {
            let dest = t as i64 + 4 + b[t + 3] as i8 as i64;
            if dest < text.raw as i64 || dest >= (text.raw + text.size) as i64 {
                continue;
            }
        }
        let lo = text.raw.max(t.saturating_sub(0xe0));
        let hi = (text.raw + text.size - 7).min(t + 0xe0);
        let matched = (lo..hi).any(|p| {
            matches!(b[p], 0x48 | 0x4c) && b[p + 1] == 0x8d && b[p + 2] & 0xc7 == 0x05 && {
                let dest = text.rva as i64
                    + (p + 7 - text.raw) as i64
                    + i32::from_le_bytes(b[p + 3..p + 7].try_into().unwrap()) as i64;
                anchors.contains(&dest)
            }
        });
        if matched {
            if b[t + 2] == 0x90 {
                already_patched += 1;
            } else {
                offsets.push(t + 2);
            }
        }
    }
    ensure!(
        (1..=8).contains(&(offsets.len() + already_patched)),
        "No unambiguous primary-anchor patch sites (or too many matches)"
    );
    Ok(PatchReport {
        offsets,
        already_patched,
        sha256: crate::platform::hash(b),
    })
}
pub fn patched(b: &[u8]) -> Result<(Vec<u8>, PatchReport)> {
    let report = inspect(b)?;
    let mut out = b.to_vec();
    for &offset in &report.offsets {
        out[offset..offset + 2].copy_from_slice(&[0x90, 0x90]);
    }
    Ok((out, report))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut b = vec![0; 0x900];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
        b[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        for (at, name, rva, raw, size) in [
            (0x188, b".text\0\0\0", 0x1000u32, 0x200u32, 0x200u32),
            (0x1b0, b".rdata\0\0", 0x2000, 0x500, 0x100),
        ] {
            b[at..at + 8].copy_from_slice(name);
            b[at + 12..at + 16].copy_from_slice(&rva.to_le_bytes());
            b[at + 16..at + 20].copy_from_slice(&size.to_le_bytes());
            b[at + 20..at + 24].copy_from_slice(&raw.to_le_bytes());
        }
        b[0x280] = 0xe8;
        b[0x281..0x285].copy_from_slice(&(-69i32).to_le_bytes());
        b[0x285..0x289].copy_from_slice(&[0x84, 0xc0, 0x74, 0x20]);
        b[0x2a0..0x2a3].copy_from_slice(&[0x48, 0x8d, 0x0d]);
        b[0x2a3..0x2a7].copy_from_slice(&0xf59i32.to_le_bytes());
        let s = b"Verified the disk integrity!\0";
        b[0x500..0x500 + s.len()].copy_from_slice(s);
        b
    }
    #[test]
    fn patch_changes_only_validated_branch_and_is_idempotent() {
        let b = fixture();
        let (out, report) = patched(&b).unwrap();
        assert_eq!(report.offsets, vec![0x287]);
        let changed: Vec<_> = b
            .iter()
            .zip(&out)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(changed, vec![0x287, 0x288]);
        let (again, report) = patched(&out).unwrap();
        assert_eq!(out, again);
        assert!(report.offsets.is_empty());
        assert_eq!(report.already_patched, 1);
    }
    #[test]
    fn rejects_a_nearby_string_without_a_valid_reference() {
        let mut b = fixture();
        b[0x2a3..0x2a7].copy_from_slice(&0i32.to_le_bytes());
        assert!(inspect(&b).is_err());
    }
    #[test]
    fn arbitrary_and_truncated_binaries_are_rejected_without_panics() {
        for len in 0..1024 {
            let mut b = vec![0; len];
            if len >= 2 {
                b[..2].copy_from_slice(b"MZ");
            }
            assert!(inspect(&b).is_err());
        }
    }
    #[test]
    fn no_binary_patch_without_anchor() {
        let mut b = vec![0; 1024];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        assert!(patched(&b).is_err());
    }
}
