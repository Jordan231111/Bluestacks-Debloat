//! Byte-preserving hosts edits. Only exact marker lines delimit owned content.
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, net::IpAddr};

pub const START: &str = "# BEGIN BLUESTACKS-DEBLOAT";
pub const END: &str = "# END BLUESTACKS-DEBLOAT";
pub const DOMAINS: &[&str] = &["ads.bluestacks.com", "adsdk.bluestacks.com"];
const LEGACY: &[&str] = &["googleads.g.doubleclick.net"];
const BOM: &[u8] = b"\xef\xbb\xbf";

struct Line<'a> {
    start: usize,
    end: usize,
    body: &'a [u8],
    newline: &'a [u8],
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t'))
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}

fn lines(input: &[u8]) -> Result<Vec<Line<'_>>> {
    ensure!(
        !input.starts_with(b"\xff\xfe")
            && !input.starts_with(b"\xfe\xff")
            && input
                .iter()
                .all(|b| *b >= 32 || matches!(b, b'\r' | b'\n' | b'\t')),
        "Unsupported hosts encoding or control bytes; the hosts file was left untouched"
    );
    let mut offset = 0;
    let mut result = Vec::new();
    for raw in input.split_inclusive(|b| *b == b'\n') {
        let newline = if raw.ends_with(b"\r\n") {
            b"\r\n".as_slice()
        } else if raw.ends_with(b"\n") {
            b"\n".as_slice()
        } else {
            b"".as_slice()
        };
        let body = &raw[..raw.len() - newline.len()];
        ensure!(
            !body.contains(&b'\r'),
            "Unsupported bare CR in hosts; the file was left untouched"
        );
        let body = if offset == 0 {
            body.strip_prefix(BOM).unwrap_or(body)
        } else {
            body
        };
        result.push(Line {
            start: offset,
            end: offset + raw.len(),
            body,
            newline,
        });
        offset += raw.len();
    }
    Ok(result)
}

fn name(domain: &str) -> Result<String> {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    ensure!(
        domain.len() <= 253
            && domain.contains('.')
            && domain.parse::<IpAddr>().is_err()
            && domain.split('.').all(|label| !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
        "Invalid hosts domain: {domain:?}"
    );
    Ok(domain.to_ascii_lowercase())
}

fn tokens(body: &[u8]) -> Vec<&[u8]> {
    body.split(|b| b.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .collect()
}

fn owned_row(body: &[u8], desired: &BTreeSet<String>) -> bool {
    // Preserve manual aliases and inline comments even inside the marked block.
    if body.contains(&b'#') {
        return false;
    }
    let fields = tokens(body);
    if fields.len() != 2 {
        return false;
    }
    let Ok(ip) = std::str::from_utf8(fields[0])
        .unwrap_or("")
        .parse::<IpAddr>()
    else {
        return false;
    };
    let Some(domain) = std::str::from_utf8(fields[1])
        .ok()
        .and_then(|s| name(s).ok())
    else {
        return false;
    };
    ip.is_unspecified()
        && (desired.contains(&domain)
            || DOMAINS.contains(&domain.as_str())
            || LEGACY.contains(&domain.as_str()))
}

fn existing_rules(
    body: &[u8],
    desired: &BTreeSet<String>,
    covered: &mut BTreeSet<(String, bool)>,
) -> Result<()> {
    let body = body.split(|b| *b == b'#').next().unwrap_or_default();
    let fields = tokens(body);
    if fields.len() < 2 {
        return Ok(());
    }
    for alias in &fields[1..] {
        let Some(domain) = std::str::from_utf8(alias).ok().and_then(|s| name(s).ok()) else {
            continue;
        };
        if !desired.contains(&domain) {
            continue;
        }
        let address = std::str::from_utf8(fields[0]).ok().and_then(|s| s.parse::<IpAddr>().ok())
            .with_context(|| format!("An existing hosts entry for {domain} has an unrecognized address; preserved without changes"))?;
        let blocking = address.is_unspecified()
            || address.is_loopback()
            || match address {
                IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| v4.is_unspecified() || v4.is_loopback()),
                _ => false,
            };
        ensure!(
            blocking,
            "An existing hosts entry maps {domain} to {address}; preserved without adding a conflicting duplicate"
        );
        covered.insert((domain, address.is_ipv6()));
    }
    Ok(())
}

pub fn update(input: &[u8], domains: &[&str]) -> Result<Vec<u8>> {
    let desired = domains
        .iter()
        .map(|domain| name(domain))
        .collect::<Result<BTreeSet<_>>>()?;
    let lines = lines(input)?;
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| (trim(l.body) == START.as_bytes()).then_some(i))
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| (trim(l.body) == END.as_bytes()).then_some(i))
        .collect::<Vec<_>>();
    ensure!(
        starts.len() == ends.len() && starts.len() <= 1,
        "Malformed or duplicate managed hosts block; the file was left untouched"
    );
    let block = if let (Some(&start), Some(&end)) = (starts.first(), ends.first()) {
        ensure!(
            start < end,
            "Reversed managed hosts markers; the file was left untouched"
        );
        Some((start, end))
    } else {
        None
    };
    let newline = block
        .and_then(|(start, _)| (!lines[start].newline.is_empty()).then_some(lines[start].newline))
        .or_else(|| {
            lines
                .iter()
                .find(|l| !l.newline.is_empty())
                .map(|l| l.newline)
        })
        .unwrap_or(b"\r\n");
    let mut custom = Vec::new();
    let mut covered = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some((start, end)) = block {
            if index == start || index == end {
                continue;
            }
            if index > start && index < end {
                if owned_row(line.body, &desired) {
                    continue;
                }
                custom.extend_from_slice(&input[line.start..line.end]);
            }
        }
        existing_rules(line.body, &desired, &mut covered)?;
    }
    let mut generated = Vec::new();
    for domain in &desired {
        for (address, ipv6) in [("0.0.0.0", false), ("::", true)] {
            if !covered.contains(&(domain.clone(), ipv6)) {
                generated.extend_from_slice(format!("{address} {domain}").as_bytes());
                generated.extend_from_slice(newline);
            }
        }
    }
    let (start, end, trailing_newline) =
        block.map_or((input.len(), input.len(), true), |(start, end)| {
            (
                lines[start].start + usize::from(start == 0 && input.starts_with(BOM)) * BOM.len(),
                lines[end].end,
                !lines[end].newline.is_empty(),
            )
        });
    let mut output = input[..start].to_vec();
    if !generated.is_empty() || !custom.is_empty() {
        if block.is_none()
            && output.len() > usize::from(input.starts_with(BOM)) * BOM.len()
            && !output.ends_with(b"\n")
        {
            output.extend_from_slice(newline);
        }
        output.extend_from_slice(START.as_bytes());
        output.extend_from_slice(newline);
        output.extend_from_slice(&custom);
        output.extend_from_slice(&generated);
        output.extend_from_slice(END.as_bytes());
        if trailing_newline {
            output.extend_from_slice(newline);
        }
    }
    output.extend_from_slice(&input[end..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_bytes_bom_comments_newlines_and_missing_final_newline() {
        for newline in ["\n", "\r\n"] {
            for bom in [b"".as_slice(), BOM] {
                for ending in ["", newline] {
                    let mut input = bom.to_vec();
                    input.extend_from_slice(
                        format!("# caf\u{e9}{newline}127.0.0.1 localhost{ending}").as_bytes(),
                    );
                    let output = update(&input, DOMAINS).unwrap();
                    assert!(output.starts_with(&input));
                    assert_eq!(output, update(&output, DOMAINS).unwrap());
                }
            }
        }
        let ansi = b"# caf\xe9\r\n127.0.0.1 localhost\r\n";
        assert!(update(ansi, DOMAINS).unwrap().starts_with(ansi));
    }
    #[test]
    fn exact_marker_lines_keep_embedded_marker_comments_intact() {
        let input = format!("# Documentation mentions {START} and {END}\n127.0.0.1 localhost\n");
        let output = update(input.as_bytes(), DOMAINS).unwrap();
        assert!(output.starts_with(input.as_bytes()));
        assert_eq!(output, update(&output, DOMAINS).unwrap());
    }
    #[test]
    fn replaces_in_place_preserving_custom_body_and_suffix() {
        let input = format!(
            "# prefix\r\n{START}\r\n# custom note\r\n10.0.0.9 private.example\r\n0.0.0.0 ads.bluestacks.com\r\n{END}\r\n# suffix\n127.0.0.1 local.example"
        );
        let output = update(input.as_bytes(), DOMAINS).unwrap();
        assert!(
            output.starts_with(
                format!("# prefix\r\n{START}\r\n# custom note\r\n10.0.0.9 private.example\r\n")
                    .as_bytes()
            )
        );
        assert!(output.ends_with(b"# suffix\n127.0.0.1 local.example"));
        assert_eq!(output, update(&output, DOMAINS).unwrap());
    }
    #[test]
    fn external_aliases_case_and_loopback_rules_do_not_get_duplicates() {
        let input =
            b"127.0.0.1 ADS.BLUESTACKS.COM. adsdk.bluestacks.com # keep\n::1 ads.bluestacks.com\n";
        let output = update(input, DOMAINS).unwrap();
        assert!(output.starts_with(input));
        let added = std::str::from_utf8(&output[input.len()..]).unwrap();
        assert!(!added.contains("0.0.0.0"));
        assert!(!added.contains(":: ads.bluestacks.com\n"));
        assert!(added.contains(":: adsdk.bluestacks.com\n"));
        assert_eq!(output, update(&output, DOMAINS).unwrap());
    }
    #[test]
    fn fully_covered_domains_need_no_managed_block() {
        let input = b"0.0.0.0 ads.bluestacks.com adsdk.bluestacks.com\n:: ads.bluestacks.com adsdk.bluestacks.com\n";
        assert_eq!(update(input, DOMAINS).unwrap(), input);
    }
    #[test]
    fn conflicting_manual_mapping_is_preserved_by_rejection() {
        for address in ["192.0.2.1", "2001:db8::1", "invalid-address"] {
            let input = format!("{address} ads.bluestacks.com\n");
            assert!(update(input.as_bytes(), DOMAINS).is_err());
        }
    }
    #[test]
    fn custom_aliases_and_inline_comments_inside_block_are_not_removed() {
        let input = format!(
            "{START}\n0.0.0.0 ads.bluestacks.com private.example\n:: ads.bluestacks.com # custom note\n{END}\n"
        );
        let output = String::from_utf8(update(input.as_bytes(), DOMAINS).unwrap()).unwrap();
        assert!(output.contains("0.0.0.0 ads.bluestacks.com private.example\n"));
        assert!(output.contains(":: ads.bluestacks.com # custom note\n"));
        assert_eq!(output.matches("ads.bluestacks.com").count(), 2);
    }
    #[test]
    fn duplicate_inputs_and_owned_rules_are_canonicalized() {
        let input =
            format!("{START}\n0.0.0.0 ads.bluestacks.com\n0.0.0.0 ADS.BLUESTACKS.COM\n{END}");
        let domains = ["ads.bluestacks.com", "ADS.BLUESTACKS.COM."];
        let output = String::from_utf8(update(input.as_bytes(), &domains).unwrap()).unwrap();
        assert_eq!(output.matches("0.0.0.0 ads.bluestacks.com").count(), 1);
        assert_eq!(output.matches(":: ads.bluestacks.com").count(), 1);
        assert!(!output.ends_with('\n'));
        assert_eq!(
            output.as_bytes(),
            update(output.as_bytes(), &domains).unwrap()
        );
    }
    #[test]
    fn damaged_markers_and_unsupported_encodings_never_produce_output() {
        for text in [
            START.to_owned(),
            END.to_owned(),
            format!("{END}\n{START}\n"),
            format!("{START}\n{START}\n{END}\n{END}\n"),
            format!("{START}\n{END}\n{START}\n{END}\n"),
        ] {
            assert!(update(text.as_bytes(), DOMAINS).is_err());
        }
        for bytes in [
            b"\xff\xfe".as_slice(),
            b"\xfe\xff",
            b"x\0y",
            b"x\x1ay",
            b"x\ry",
        ] {
            assert!(update(bytes, DOMAINS).is_err());
        }
    }
    #[test]
    fn invalid_domains_are_rejected_without_injection() {
        for domain in [
            "",
            "localhost",
            "a..example",
            ".example",
            "-a.example",
            "a-.example",
            "a/b.example",
            "a.example\n127.0.0.1 victim",
            "127.0.0.1",
            "::1",
            "a.ex ample",
        ] {
            assert!(update(b"", &[domain]).is_err(), "{domain:?}");
        }
        assert!(update(b"", &[&format!("{}.example", "a".repeat(64))]).is_err());
    }
    #[test]
    fn empty_domain_set_removes_only_owned_rows_and_markers() {
        let input = format!("# before\n{START}\n0.0.0.0 ads.bluestacks.com\n{END}\n# after\n");
        assert_eq!(
            update(input.as_bytes(), &[]).unwrap(),
            b"# before\n# after\n"
        );
        assert_eq!(update(b"# untouched", &[]).unwrap(), b"# untouched");
    }
    #[test]
    fn generated_unrelated_records_survive_repeated_edits() {
        for case in 0..600 {
            let newline = if case % 2 == 0 { "\r\n" } else { "\n" };
            let input = format!(
                "# case {case}: {START} is documentation{newline}10.0.{}.{} internal-{case}.example alias-{case}.example # retain{newline}",
                case / 250,
                case % 250 + 1
            );
            let output = update(input.as_bytes(), DOMAINS).unwrap();
            assert!(output.starts_with(input.as_bytes()));
            assert_eq!(output, update(&output, DOMAINS).unwrap());
        }
    }
}
