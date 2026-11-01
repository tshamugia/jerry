use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tauri-build would embed its manifest only into the app binary. Tests and examples link
    // Tauri too and, without Common Controls v6, fail to start (STATUS_ENTRYPOINT_NOT_FOUND),
    // so the linker embeds our manifest into every target of this crate instead.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("jerry.exe.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFESTUAC:NO"); // the manifest itself says asInvoker

    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))?;
    Ok(())
}
