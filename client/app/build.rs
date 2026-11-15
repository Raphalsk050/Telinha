//! Gives the program its version, its icon, and, in a release build, the capture engine inside it.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("..").join("..");

    // The version lives in version.json at the root, which only people change.
    let version_file = root.join("version.json");
    println!("cargo:rerun-if-changed={}", version_file.display());
    let version = std::fs::read_to_string(&version_file)
        .ok()
        .and_then(|text| {
            let start = text.find("\"version\"")?;
            let rest = &text[start + 9..];
            let open = rest.find('"')? + 1;
            let close = rest[open..].find('"')? + open;
            Some(rest[open..close].to_owned())
        })
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap());
    println!("cargo:rustc-env=TELINHA_VERSION={version}");

    // A release build carries the engine, so the app is one file. A development build finds
    // the engine in the build folder of the repository.
    println!("cargo:rustc-check-cfg=cfg(embedded_engine)");
    println!("cargo:rerun-if-env-changed=TELINHA_ENGINE");
    let engine = std::env::var_os("TELINHA_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("build").join("msvc-release").join("tools").join("telinha").join("Release").join("telinha.exe"));
    println!("cargo:rerun-if-changed={}", engine.display());
    if std::env::var("PROFILE").as_deref() == Ok("release") && engine.is_file() {
        let engine = std::fs::canonicalize(&engine).unwrap();
        println!("cargo:rustc-cfg=embedded_engine");
        println!("cargo:rustc-env=TELINHA_ENGINE_FILE={}", engine.display());
    }

    // Only the program the release pipeline builds updates itself. One built by hand would be
    // swapped for whatever the latest release is, which may not even be this app.
    println!("cargo:rustc-check-cfg=cfg(self_update)");
    println!("cargo:rerun-if-env-changed=TELINHA_UPDATES");
    if std::env::var("TELINHA_UPDATES").as_deref() == Ok("1") {
        println!("cargo:rustc-cfg=self_update");
    }

    icon(&root.join("desktop").join("assets").join("icon.ico"), &version);
}

#[cfg(windows)]
fn icon(file: &Path, version: &str) {
    println!("cargo:rerun-if-changed={}", file.display());
    let mut resource = winresource::WindowsResource::new();
    if file.is_file() {
        resource.set_icon(&file.to_string_lossy());
    }
    resource.set("ProductName", "Telinha");
    resource.set("FileDescription", "Telinha");
    resource.set("ProductVersion", version);
    resource.set("FileVersion", version);
    if let Err(error) = resource.compile() {
        println!("cargo:warning=sem icone no executavel: {error}");
    }
}

#[cfg(not(windows))]
fn icon(_file: &Path, _version: &str) {}
