fn main() {
    println!("cargo:rerun-if-changed=../windows-app-manifest.xml");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    tauri_winres::WindowsResource::new()
        .set_manifest(include_str!("../windows-app-manifest.xml"))
        .compile_for(&["micyou-cli"])
        .expect("failed to embed the Windows Common Controls v6 manifest into micyou-cli");
}
