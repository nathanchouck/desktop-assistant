use std::env;
use std::path::PathBuf;

fn main() {
    // The app manifest (Common Controls v6) is embedded through the linker
    // below instead of by tauri-build, so test executables get it too.
    // Without it, Windows test binaries crash on start with
    // STATUS_ENTRYPOINT_NOT_FOUND. Same workaround as Tauri's own repo.
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("failed to run tauri-build");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // Turn linker warnings (e.g. a bad manifest) into errors.
        println!("cargo:rustc-link-arg=/WX");
    }

    if target_os == "windows" {
        build_everything_sdk();
    }
}

/// Compiles the vendored Everything SDK (file search) into a static library.
fn build_everything_sdk() {
    let sdk = PathBuf::from("third_party/everything-sdk");
    println!("cargo:rerun-if-changed={}", sdk.display());
    cc::Build::new()
        .file(sdk.join("src/Everything.c"))
        .define("UNICODE", None)
        .define("_UNICODE", None)
        // Third-party code: its warnings aren't ours to fix.
        .warnings(false)
        .compile("everything_sdk");
    println!("cargo:rustc-link-lib=user32");
}
