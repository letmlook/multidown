fn main() {
    embed_windows_manifest();

    let attributes = tauri_build::Attributes::new();
    // On Windows the manifest is owned by `embed_windows_manifest` below, so tauri-build
    // must not add a second one: two `MANIFEST/1` resources in the same link are a hard
    // `CVT1100: duplicate resource` / `LNK1123` failure.
    #[cfg(target_os = "windows")]
    let attributes =
        attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());

    // Same contract as `tauri_build::build()`, which takes no attributes.
    if let Err(error) = tauri_build::try_build(attributes) {
        println!("{error:#}");
        std::process::exit(1);
    }
}

// `cargo test` / `cargo build` on Windows could not *start* their binaries: every one of
// them imports `comctl32!TaskDialogIndirect`, which only exists in Common Controls v6, and
// a Rust test harness embeds no SxS manifest requesting it. `comctl32` therefore bound to
// the legacy v5 and the process died before `main` with `0xC0000139`
// (STATUS_ENTRYPOINT_NOT_FOUND).
//
// tauri-build already links exactly the right manifest, but it scopes it with
// `cargo:rustc-link-arg-bins`, which Cargo applies to bin and `tests/*.rs` targets only —
// not to the `cargo test --lib` unit. That unit's binary was the one left without a
// manifest. `cargo:rustc-link-arg-tests` does not reach it either (verified: the resource
// is still absent and the binary still exits 0xC0000139), and the unscoped
// `cargo:rustc-link-arg` is the only scope that covers it, so the manifest is linked here
// with that one instead.
//
// `manifest_required` on purpose: a manifest that silently fails to embed is worse than
// none, because the job goes green while every test binary still dies at load time.
//
// The `cfg(windows)` build-dependency and this `cfg` both key off the *host*, which is
// consistent because the project is never cross-compiled to Windows from another OS.
#[cfg(target_os = "windows")]
fn embed_windows_manifest() {
    println!("cargo:rerun-if-changed=windows/common-controls.rc");
    embed_resource::compile_for_everything("windows/common-controls.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the Windows Common Controls v6 manifest");
}

#[cfg(not(target_os = "windows"))]
fn embed_windows_manifest() {}
