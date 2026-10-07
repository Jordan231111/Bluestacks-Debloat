# Root payload provenance

These payloads were imported from the user's current `BluestacksRoot/blueStackRoot.cmd` using `tools/import-root-assets.ps1`. No BlueStacks proprietary executable or virtual disk is embedded.

- `magisk.apk`: the companion's custom Kitsune Mask 31 build, SHA-256 `fac319d2de262fcfff1684e13e1a5c61c486d2a773a7a8ffcfdbfe6f763a7fd4`. Corresponding source: [Jordan231111/KitsuneMagisk](https://github.com/Jordan231111/KitsuneMagisk/tree/25fa2159f), derived from [KitsuneMagisk](https://github.com/1q23lyc45/KitsuneMagisk). Its own GPL license applies.
- `debugfs.zip`: the companion's Cygwin debugfs 1.44.5 bundle, SHA-256 `008b6006e766d2591c8c7db7bf6d6a0a4b9cd6116b9a8e2737151828eb577632`. The e2fsprogs and Cygwin components retain their respective licenses; source projects: [e2fsprogs](https://git.kernel.org/pub/scm/fs/ext2/e2fsprogs.git/tag/?h=v1.44.5), [Cygwin](https://cygwin.com/).
- `bootstrap.gz`: compressed companion bootstrap; decoded ELF is 4,968 bytes with SHA-256 `7eb6380ee26ce0b68d9f3f23ac04f50e0dfdd49359ef17d1a4978be1795913dd`. Corresponding source is included as `bsr_su.c`.
- Guest shell/init templates are carried over from the current companion's Magisk workflow. Host orchestration, disk access, configuration, backups and validation are implemented in Rust.

The embedded components are separate tools and retain their original licenses. These payloads and templates come from the user-supplied companion distribution; the root host engine is a native Rust implementation.

License texts accompany releases under `assets/licenses`. The debugfs bundle also contains e2fsprogs libraries, Cygwin 3.6.9 (`cygwin1.dll`), GCC's `cyggcc_s-seh-1.dll`, GNU libiconv (`cygiconv-2.dll`), GNU gettext (`cygintl-8.dll`), and the UUID library (`cyguuid-1.dll`). Their upstream source projects are [Cygwin](https://cygwin.com/git/newlib-cygwin.git), [GCC](https://gcc.gnu.org/git.html), [GNU libiconv](https://www.gnu.org/software/libiconv/), [GNU gettext](https://www.gnu.org/software/gettext/), and [util-linux](https://github.com/util-linux/util-linux). Component licenses are independent of the repository's license. The release's `THIRD-PARTY-NOTICES.txt` also includes Rust dependency and bundled font notices.
