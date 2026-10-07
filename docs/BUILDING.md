# Build from source

End users only need the [release executable](https://github.com/Jordan231111/Bluestacks-Debloat/releases/latest). These steps are for developers.

## Requirements

- Windows x64.
- [rustup](https://rust-lang.org/tools/install/).
- [MSVC C++ Build Tools and the Windows SDK](https://rust-lang.github.io/rustup/installation/windows-msvc.html).

Clone the repository and run from its root:

```powershell
.\tools\build.ps1
```

The repository pins `nightly-2026-10-07`, uses Rust edition 2024, and locks dependencies in `Cargo.lock`. No unstable Rust language features are used. The MSVC CRT is linked statically.

The build runs formatting checks, Clippy, tests, and an optimized build. Outputs go to `dist/`, including `BluestacksDebloat.exe`, documentation, license notices, and `SHA256SUMS.txt`.

## Individual checks

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

Root payloads are checked into `assets/root` and verified by checksum. Normal builds do not require a local checkout of BluestacksRoot. The import script is only for maintainers updating those payloads; see [their provenance](../assets/root/NOTICE.md).

The Windows GitHub Actions workflow performs the same checks and uploads a build artifact. Releases are published separately after validation.
