//! Embeds the icon, version information and the application manifest into
//! the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/swiftdm.ico");
    println!("cargo:rerun-if-changed=assets/swiftdm.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/swiftdm.ico")
        .set_manifest_file("assets/swiftdm.manifest")
        .set("ProductName", "SwiftDM")
        .set("FileDescription", "SwiftDM")
        .set("CompanyName", "Bashar Salmo")
        .set("LegalCopyright", "© 2026 Bashar Salmo. Powered by Bashar Salmo.")
        .set("OriginalFilename", "swiftdm-gui.exe");
    // Cross-compiling without a resource compiler still produces a working
    // (icon-less) binary; release builds on Windows always have one.
    if let Err(e) = res.compile() {
        println!("cargo:warning=Windows resources not embedded: {e}");
    }
}
