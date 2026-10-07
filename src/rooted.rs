//! Optional per-instance Magisk module. No system remount or shared VHD edits.
use crate::{
    adb::{Client, quote},
    network,
    transaction::{Operation, Target, Value},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
pub const MODULE: &str = "/data/adb/modules/bluestacks_debloat";
const PROP: &str = "id=bluestacks_debloat\nname=BlueStacks Debloat\nversion=1.0\nversionCode=1\nauthor=BlueStacks Debloat\ndescription=Optional hosts filtering and launcher-only network isolation\n";
const CHAIN: &str = "BSD_LAUNCHER_V1";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Policy {
    version: u8,
    hosts: Option<String>,
    isolate_launcher: bool,
}
fn require_root(adb: &Client) -> Result<()> {
    ensure!(
        adb.root("id -u")?.trim() == "0",
        "This option requires working Magisk root"
    );
    ensure!(
        !adb.root("magisk -v")?.trim().is_empty(),
        "Magisk is required to keep these changes across reboots"
    );
    Ok(())
}
fn launcher_uid(adb: &Client) -> Result<u32> {
    let dump = adb.shell("dumpsys package com.uncube.launcher3")?;
    let re = regex::Regex::new(r"(?m)^\s*userId=(\d+)\s*$")?;
    let uid = re
        .captures(&dump)
        .ok_or_else(|| anyhow::anyhow!("Launcher UID could not be read"))?[1]
        .parse::<u32>()?;
    ensure!(
        uid >= 10000,
        "Refusing to filter a privileged/shared system UID"
    );
    let list = adb.shell("pm list packages -U --user 0")?;
    ensure!(
        list.lines()
            .filter(|line| line.trim().ends_with(&format!(" uid:{uid}")))
            .count()
            == 1,
        "The launcher shares a UID; network isolation is not safe on this build"
    );
    Ok(uid)
}
fn service(policy: &Policy) -> String {
    if !policy.isolate_launcher {
        return "#!/system/bin/sh\n# Hosts are mounted by Magisk on boot.\n".into();
    }
    format!(
        r#"#!/system/bin/sh
# BlueStacks Debloat owns only the {CHAIN} chain. Never flush other chains.
until [ "$(getprop sys.boot_completed)" = "1" ]; do sleep 2; done
uid=$(dumpsys package com.uncube.launcher3 | sed -n 's/^[[:space:]]*userId=\([0-9]*\).*$/\1/p' | head -n 1)
case "$uid" in ''|*[!0-9]*) exit 1;; esac
[ "$uid" -ge 10000 ] || exit 1
[ "$(pm list packages -U --user 0 | grep -c " uid:$uid$")" = "1" ] || exit 1
for tool in iptables ip6tables; do
  "$tool" -w 3 -N {CHAIN} 2>/dev/null || true
  "$tool" -w 3 -F {CHAIN} || exit 1
  if [ "$tool" = "iptables" ]; then
    "$tool" -w 3 -A {CHAIN} -m owner --uid-owner "$uid" -d 127.0.0.0/8 -j RETURN || exit 1
    "$tool" -w 3 -A {CHAIN} -m owner --uid-owner "$uid" -d 10.0.2.2/32 -j RETURN || exit 1
  else
    "$tool" -w 3 -A {CHAIN} -m owner --uid-owner "$uid" -d ::1/128 -j RETURN || exit 1
  fi
  "$tool" -w 3 -A {CHAIN} -m owner --uid-owner "$uid" -j REJECT || exit 1
  "$tool" -w 3 -C OUTPUT -j {CHAIN} 2>/dev/null || "$tool" -w 3 -I OUTPUT 1 -j {CHAIN} || exit 1
done
"#
    )
}
pub fn read(adb: &Client) -> Result<Value> {
    require_root(adb)?;
    let json = adb.root(&format!(
        "if [ -d {MODULE} ]; then cat {MODULE}/policy.json; else printf 'ABSENT'; fi"
    ))?;
    if json == "ABSENT" {
        return Ok(Value::Missing);
    }
    let policy: Policy = serde_json::from_str(&json)?;
    ensure!(policy.version == 1, "Unsupported managed module version");
    ensure!(
        adb.root(&format!("cat {MODULE}/module.prop"))?.trim_end() == PROP.trim_end(),
        "Existing module is not owned by this version of BlueStacks Debloat"
    );
    ensure!(
        adb.root(&format!("cat {MODULE}/service.sh"))?.trim_end() == service(&policy).trim_end(),
        "Managed network script was modified; preserving it"
    );
    if let Some(hosts) = &policy.hosts {
        ensure!(
            adb.root(&format!("cat {MODULE}/system/etc/hosts"))?
                .trim_end()
                == hosts.trim_end(),
            "Managed hosts file was modified; preserving it"
        );
    }
    Ok(Value::Bytes(serde_json::to_vec(&policy)?))
}
pub fn write(adb: &Client, value: &Value) -> Result<()> {
    adb.verify_identity()?;
    require_root(adb)?;
    match value {
        Value::Missing => {
            if read(adb)? == Value::Missing {
                return Ok(());
            }
            // Remove only our chain; the hosts overlay disappears after a reboot.
            adb.root(&format!("for tool in iptables ip6tables; do while $tool -w 3 -C OUTPUT -j {CHAIN} 2>/dev/null; do $tool -w 3 -D OUTPUT -j {CHAIN} || exit 1; done; if $tool -w 3 -S {CHAIN} >/dev/null 2>&1; then $tool -w 3 -F {CHAIN} && $tool -w 3 -X {CHAIN} || exit 1; fi; done; test -f {MODULE}/policy.json && rm -rf {MODULE}"))?;
        }
        Value::Bytes(bytes) => {
            let policy: Policy = serde_json::from_slice(bytes)?;
            ensure!(policy.version == 1, "Unsupported module policy");
            let previous = read(adb)?;
            if policy.isolate_launcher {
                launcher_uid(adb)?;
            }
            let stage = format!("/data/adb/modules/.bsd-{}", uuid::Uuid::new_v4());
            let old = format!("{stage}-previous");
            let mut script = format!(
                "set -e\ntest ! -e {stage}\nmkdir -p {stage}/system/etc\ntrap 'rm -rf {stage}' EXIT\nprintf %s {} > {stage}/module.prop\nprintf %s {} > {stage}/service.sh\nprintf %s {} > {stage}/policy.json\nchmod 0755 {stage}/service.sh\n",
                quote(PROP),
                quote(&service(&policy)),
                quote(std::str::from_utf8(bytes)?)
            );
            if let Some(hosts) = &policy.hosts {
                script.push_str(&format!("printf %s {} > {stage}/system/etc/hosts\nchmod 0644 {stage}/system/etc/hosts\n",quote(hosts)));
            }
            if previous != Value::Missing {
                script.push_str(&format!("test ! -e {old}\nmv {MODULE} {old}\nif ! mv {stage} {MODULE}; then mv {old} {MODULE}; exit 1; fi\nrm -rf {old}\ntrap - EXIT\n"));
            } else {
                script.push_str(&format!(
                    "test ! -e {MODULE}\nmv {stage} {MODULE}\ntrap - EXIT\n"
                ));
            }
            adb.root(&script)?;
            if policy.isolate_launcher {
                adb.root(&format!("sh {MODULE}/service.sh"))?;
                let uid = launcher_uid(adb)?;
                for tool in ["iptables", "ip6tables"] {
                    adb.root(&format!("{tool} -w 3 -C OUTPUT -j {CHAIN} && {tool} -w 3 -C {CHAIN} -m owner --uid-owner {uid} -j REJECT"))?;
                }
                // Reboot after applying to refresh the launcher and activate hosts.
                // Some vendor builds reset the ADB transport when their launcher is
                // force-stopped; avoid doing that inside a verified transaction.
            } else if previous != Value::Missing {
                adb.root(&format!("for tool in iptables ip6tables; do while $tool -w 3 -C OUTPUT -j {CHAIN} 2>/dev/null; do $tool -w 3 -D OUTPUT -j {CHAIN} || exit 1; done; if $tool -w 3 -S {CHAIN} >/dev/null 2>&1; then $tool -w 3 -F {CHAIN} && $tool -w 3 -X {CHAIN} || exit 1; fi; done"))?;
            }
        }
        _ => anyhow::bail!("Invalid module state"),
    }
    Ok(())
}
pub fn operation(adb: &Client, hosts: bool, isolate: bool) -> Result<Option<Operation>> {
    require_root(adb)?;
    let before = read(adb)?;
    if isolate {
        launcher_uid(adb)?;
        for tool in ["iptables", "ip6tables"] {
            if before == Value::Missing {
                adb.root(&format!("set -e; {tool} -w 3 -S OUTPUT >/dev/null; if {tool} -w 3 -S {CHAIN} >/dev/null 2>&1; then exit 1; fi"))?;
            }
        }
    }
    let hosts = if hosts {
        let current = adb.shell("cat /system/etc/hosts")?;
        Some(String::from_utf8(network::hosts_block(
            format!("{current}\n").as_bytes(),
            network::DOMAINS,
        )?)?)
    } else {
        None
    };
    let policy = Policy {
        version: 1,
        hosts,
        isolate_launcher: isolate,
    };
    let after = Value::Bytes(serde_json::to_vec(&policy)?);
    if before == after {
        return Ok(None);
    }
    Ok(Some(Operation {
        label: format!(
            "Install per-instance Magisk module: Android hosts={}, launcher network isolation={isolate}",
            policy.hosts.is_some()
        ),
        target: Target::RootModule,
        before,
        after,
    }))
}
