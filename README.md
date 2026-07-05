# Bluestacks-Debloat WIP DO NOT USE YET

<p align="center">
  <a href="https://github.com/Jordan231111/Bluestacks-Debloat/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/Jordan231111/Bluestacks-Debloat?style=flat&logo=github"></a>
  <a href="https://github.com/Jordan231111/Bluestacks-Debloat/network/members"><img alt="Forks" src="https://img.shields.io/github/forks/Jordan231111/Bluestacks-Debloat?style=flat&logo=github"></a>
  <img alt="BlueStacks" src="https://img.shields.io/badge/BlueStacks%205-5.22%2B%20%E2%9C%93-blue">
  <img alt="Ads" src="https://img.shields.io/badge/ads-removed-success">
  <img alt="Tracking" src="https://img.shields.io/badge/tracking-blocked-success">
  <a href="./LICENSE"><img alt="License" src="https://img.shields.io/badge/license-CC%20BY--NC--ND%204.0-lightgrey"></a>
</p>

**Debloat BlueStacks 5 — remove the ads, tracking, telemetry and bloatware from one file, and keep them gone.**
Strip out the in-app ads, the start-up ad, the "click for rewards" nags, the App Center / game
recommendations, the notification spam, the tracking/telemetry domains, and the preinstalled junk apps —
then reclaim the RAM and CPU they were burning. Built for the **latest BlueStacks 5 (5.22+)**, where most
older ad-removal tricks now get your instance **killed for "tampering."** This one is designed to survive that.

> **Why another debloater?** Every other method is either outdated (BlueStacks 4 / Android 7), a full
> re-packaged installer of an *old* BlueStacks build, or a hosts-file hack that **stops working on 5.22+**
> because BlueStacks detects the change and terminates the instance. `Bluestacks-Debloat` is a current,
> script-based, *non-redistributing* debloater that pairs with the proven
> [HD-Player anti-tamper patch](https://github.com/Jordan231111/BluestacksRoot) so your changes actually stick.

---

## ⚡ Quick Start

1. **Close** BlueStacks completely (including the Multi-Instance Manager).
2. **Right-click `blueStackDebloat.cmd` → Run as administrator.** (If Windows SmartScreen shows
   *"Windows protected your PC"*, click **More info → Run anyway** — see [Is this safe?](#-is-this-safe) below.)
3. Pick what to strip from the menu, or choose **Full Debloat** to do everything. Re-launch BlueStacks — clean.

No account, no subscription, no re-install, nothing extra to download.

---

## 🧹 What it removes

| Category | What goes away | Side |
|---|---|---|
| **In-app & start-up ads** | The banner ads, the ad that plays when an instance boots, gameplay ad overlays | Host (`bluestacks.conf`) |
| **"Click for rewards" / promos** | The rewards nag, promoted-app popups, install-app campaigns | Host + Guest |
| **App Center / recommendations** | The game-recommendation wall and "discover" promos | Guest |
| **Tracking & telemetry** | Ad/analytics/telemetry domains null-routed at the guest `hosts` file | Guest |
| **Notification spam** | The promotional notifications and "tips" | Host + Guest |
| **Helper bloat processes** | Background helper/agent processes you didn't ask for | Host |
| **Preinstalled junk apps** | The bundled junk APKs (`pm disable-user` / `uninstall --user 0`) | Guest |

Every change is **backed up first** and is **fully reversible** — see [Undo](#-undo--restore).
Full itemized list: **[`docs/WHAT-IT-REMOVES.md`](docs/WHAT-IT-REMOVES.md)** · Walkthrough & rollback:
**[`docs/RUNBOOK.md`](docs/RUNBOOK.md)**.

---

## 🛡️ Is this safe?

**Yes — and your antivirus / SmartScreen may still warn you, which is a false positive common to every
emulator-modding tool** (an unsigned script that edits an emulator). Here's why you can trust it:

- **100% open source.** Every line is plain, readable PowerShell + batch, right here in this repo. Nothing
  is obfuscated or encrypted.
- **It doesn't redistribute BlueStacks.** Unlike the "non-bloated" repacks, this never ships a modified
  BlueStacks binary — it edits *your own* installation and can put it back.
- **Everything is backed up and reversible.** `bluestacks.conf`, the guest `hosts` file, and the list of
  disabled packages are saved before any change; the **Undo** option restores them.
- **Verify it yourself.** Read [`blueStackDebloat.ps1`](blueStackDebloat.ps1), or scan the download on
  [VirusTotal](https://www.virustotal.com/).

---

## 🎥 Video tutorial

> 📺 **Watch the full debloat guide:** _link coming with the video release_ — it walks through every step,
> shows a live before/after ad + speed test, and how to undo.

<!-- Replace with the embed once published:
[![Debloat BlueStacks 5 — Remove Ads, Tracking & Bloatware](https://img.youtube.com/vi/VIDEO_ID/maxresdefault.jpg)](https://youtu.be/VIDEO_ID)
-->

---

## 🔁 Survives the latest BlueStacks anti-tampering

BlueStacks 5.22+ watches its disk and config and will terminate an instance it thinks was modified — which
is exactly why the popular hosts-file ad-block gists now break. `Bluestacks-Debloat` is built to work *with*
the version-proof **HD-Player integrity patch** from the companion
[**BluestacksRoot**](https://github.com/Jordan231111/BluestacksRoot) project, so debloat edits to the guest
disk persist instead of getting reverted or killed. Host-side `bluestacks.conf` and process changes work
on their own; the guest-disk parts (hosts file, removing system junk) are the ones that benefit from the patch.

---

## ❓ FAQ

- **The ads came back after an update — why?** A BlueStacks update can rewrite `bluestacks.conf` and restore
  defaults. Re-run the tool; it's idempotent.
- **Will this get my instance banned?** It edits your local emulator only; it doesn't touch any game or
  account. As with all emulator mods, use at your own risk.
- **Does it work on Android 9 / 11 / 13 instances?** Yes — host-side changes are version-independent; the
  guest package list adapts to what's actually installed.
- **Do I need root?** No for the host-side ad/telemetry removal. Removing some *system* preinstalled apps
  cleanly is easier on a rooted instance (see [BluestacksRoot](https://github.com/Jordan231111/BluestacksRoot)).
- **Can I undo?** Yes — see below.

## ↩️ Undo / Restore

Run the tool and choose **Undo**. It restores your backed-up `bluestacks.conf`, restores the guest `hosts`
file, and re-enables any packages it disabled. See [`docs/RUNBOOK.md`](docs/RUNBOOK.md) for the manual steps.

## 🔗 Related

- 🔓 **Root BlueStacks 5 with real Magisk:** [**BluestacksRoot**](https://github.com/Jordan231111/BluestacksRoot)
  — the companion project. Debloat first, then root.

## ☕ Support

If this saved you time, a coffee keeps the open-source emulator tools coming:
- https://ko-fi.com/yejordan
- https://buymeacoffee.com/yejordan

## 📄 License

Licensed under the Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International License.
See [LICENSE](./LICENSE) or visit http://creativecommons.org/licenses/by-nc-nd/4.0/.

---

<sub>Keywords: debloat BlueStacks, BlueStacks debloat, remove BlueStacks ads, BlueStacks no ads, disable
BlueStacks ads, BlueStacks remove ads 2026, BlueStacks bloatware, BlueStacks ad blocker, BlueStacks tracking,
BlueStacks telemetry, BlueStacks click for rewards, remove BlueStacks App Center, make BlueStacks faster,
debloat Android emulator, BlueStacks 5 ads won't turn off.</sub>
