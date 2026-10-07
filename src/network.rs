use crate::{
    discovery::{self, Installation},
    platform,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
};
use windows_sys::Win32::{
    Foundation::ERROR_INSUFFICIENT_BUFFER,
    NetworkManagement::IpHelper::*,
    Networking::WinSock::{AF_INET, AF_INET6},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub pid: u32,
    pub local_ip: String,
    pub local_port: u16,
    pub remote_ip: String,
    pub remote_port: u16,
    pub state: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub sampled_at: String,
    pub connections: Vec<Connection>,
    pub log_domains: Vec<String>,
    pub explanation: String,
}

pub fn connections() -> Result<Vec<Connection>> {
    let mut out = Vec::new();
    for family in [AF_INET, AF_INET6] {
        let mut size = 0;
        let first = unsafe {
            GetExtendedTcpTable(
                std::ptr::null_mut(),
                &mut size,
                0,
                family as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        ensure!(
            first == ERROR_INSUFFICIENT_BUFFER || first == 0,
            "Cannot size TCP table: {first}"
        );
        let mut buf = vec![0u32; (size as usize).div_ceil(4).max(1)];
        let mut status = unsafe {
            GetExtendedTcpTable(
                buf.as_mut_ptr().cast(),
                &mut size,
                0,
                family as u32,
                TCP_TABLE_OWNER_PID_ALL,
                0,
            )
        };
        // Connections may be created between the sizing and retrieval calls.
        for _ in 0..3 {
            if status != ERROR_INSUFFICIENT_BUFFER {
                break;
            }
            buf.resize((size as usize).div_ceil(4), 0);
            status = unsafe {
                GetExtendedTcpTable(
                    buf.as_mut_ptr().cast(),
                    &mut size,
                    0,
                    family as u32,
                    TCP_TABLE_OWNER_PID_ALL,
                    0,
                )
            };
        }
        ensure!(status == 0, "Cannot read TCP table: {status}");
        let count = buf[0] as usize;
        let ptr = unsafe { buf.as_ptr().cast::<u8>().add(4) };
        let row_size = if family == AF_INET {
            std::mem::size_of::<MIB_TCPROW_OWNER_PID>()
        } else {
            std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>()
        };
        ensure!(4 + count * row_size <= buf.len() * 4, "Truncated TCP table");
        for i in 0..count {
            let row = unsafe { ptr.add(i * row_size) };
            let c = if family == AF_INET {
                let r = unsafe { std::ptr::read_unaligned(row.cast::<MIB_TCPROW_OWNER_PID>()) };
                Connection {
                    pid: r.dwOwningPid,
                    local_ip: Ipv4Addr::from(r.dwLocalAddr.to_ne_bytes()).to_string(),
                    local_port: u16::from_be(r.dwLocalPort as u16),
                    remote_ip: Ipv4Addr::from(r.dwRemoteAddr.to_ne_bytes()).to_string(),
                    remote_port: u16::from_be(r.dwRemotePort as u16),
                    state: r.dwState,
                }
            } else {
                let r = unsafe { std::ptr::read_unaligned(row.cast::<MIB_TCP6ROW_OWNER_PID>()) };
                Connection {
                    pid: r.dwOwningPid,
                    local_ip: Ipv6Addr::from(r.ucLocalAddr).to_string(),
                    local_port: u16::from_be(r.dwLocalPort as u16),
                    remote_ip: Ipv6Addr::from(r.ucRemoteAddr).to_string(),
                    remote_port: u16::from_be(r.dwRemotePort as u16),
                    state: r.dwState,
                }
            };
            out.push(c);
        }
    }
    Ok(out)
}
pub fn report(install: &Installation) -> Result<Report> {
    let pids: BTreeSet<_> = discovery::processes(install)?
        .into_iter()
        .map(|p| p.pid)
        .collect();
    let connections = connections()?
        .into_iter()
        .filter(|c| pids.contains(&c.pid) && c.remote_port != 0)
        .collect();
    let re = regex::Regex::new(r"https?://([A-Za-z0-9.-]+)")?;
    let log_domains: BTreeSet<_> = re
        .captures_iter(&discovery::log_tail(install).unwrap_or_default())
        .map(|c| c[1].to_ascii_lowercase())
        .collect();
    Ok(Report {sampled_at:chrono::Utc::now().to_rfc3339(),connections,log_domains:log_domains.into_iter().collect(),explanation:"TCP endpoints belong to the selected BlueStacks installation; domains are independently observed in its recent log. An IP address is not proof of an ad endpoint. Shared CDNs, Google login, Play Store and game servers are not automatically blocked. UDP/QUIC and encrypted DNS are not classified by this snapshot.".into()})
}
pub fn hosts_path() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32/drivers/etc/hosts")
}
pub const START: &str = "# BEGIN BLUESTACKS-DEBLOAT";
pub const END: &str = "# END BLUESTACKS-DEBLOAT";
pub const DOMAINS: &[&str] = &["ads.bluestacks.com", "adsdk.bluestacks.com"];
pub fn hosts_block(input: &[u8], domains: &[&str]) -> Result<Vec<u8>> {
    let original = std::str::from_utf8(input)?;
    let nl = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let starts: Vec<_> = original.match_indices(START).collect();
    let ends: Vec<_> = original.match_indices(END).collect();
    ensure!(
        starts.len() == ends.len() && starts.len() <= 1,
        "Malformed or duplicate managed hosts block"
    );
    let mut output = original.to_owned();
    if let (Some((start, _)), Some((end, _))) = (starts.first(), ends.first()) {
        ensure!(start < end, "Malformed managed hosts block");
        let suffix = *end + END.len();
        let suffix = suffix
            + if original[suffix..].starts_with("\r\n") {
                2
            } else if original[suffix..].starts_with('\n') {
                1
            } else {
                0
            };
        output.replace_range(*start..suffix, "");
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(nl);
    }
    output.push_str(START);
    output.push_str(nl);
    for d in domains {
        ensure!(
            d.contains('.')
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                && d.parse::<IpAddr>().is_err(),
            "Invalid domain"
        );
        output.push_str(&format!("0.0.0.0 {d}{nl}:: {d}{nl}"));
    }
    output.push_str(END);
    output.push_str(nl);
    Ok(output.into_bytes())
}
pub fn export(install: &Installation) -> Result<PathBuf> {
    let dir = platform::state_dir().join("reports");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!(
        "network-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    platform::atomic_write(&path, &serde_json::to_vec_pretty(&report(install)?)?)?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosts_preserves_unrelated_and_is_idempotent() {
        let input = b"127.0.0.1 localhost\r\n1.2.3.4 internal.example\r\n";
        let a = hosts_block(input, DOMAINS).unwrap();
        assert!(a.starts_with(input));
        assert_eq!(a, hosts_block(&a, DOMAINS).unwrap());
    }
    #[test]
    fn broken_markers_rejected() {
        assert!(hosts_block(START.as_bytes(), DOMAINS).is_err());
    }
    #[test]
    fn migration_removes_legacy_google_rules_only_from_owned_block() {
        let input = format!(
            "127.0.0.1 localhost\n1.2.3.4 my-service.example\n{START}\n0.0.0.0 googleads.g.doubleclick.net\n0.0.0.0 ads.bluestacks.com\n{END}\n"
        );
        let result = String::from_utf8(hosts_block(input.as_bytes(), DOMAINS).unwrap()).unwrap();
        assert!(!result.contains("doubleclick"));
        assert!(result.contains("ads.bluestacks.com"));
        assert!(result.contains("1.2.3.4 my-service.example"));
        assert!(DOMAINS.iter().all(|d| d.ends_with(".bluestacks.com")));
    }
}
