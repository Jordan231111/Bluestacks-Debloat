fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "BlueStacks Debloat")
            .set(
                "FileDescription",
                "BlueStacks Debloat — native Rust utility",
            )
            .set("LegalCopyright", "CC BY-NC-ND 4.0; see LICENSE")
            .set_manifest_file("app.manifest");
        res.compile().expect("Windows resource compilation failed");
    }
}
