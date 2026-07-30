// Build script for the installer shell.
//
// Two jobs: the usual Tauri codegen, and embedding the NSIS installer that this
// shell drives. The NSIS installer is baked in with `include_bytes!` so the
// shipped Brume-Setup.exe is a single self-contained file rather than something
// that has to travel alongside a second executable.
//
// This creates a build ordering requirement: the browser must be bundled before
// this crate is compiled, because the artifact being embedded is the output of
// `tauri build`. tools/build-installer.ps1 runs the two stages in order.

use std::{env, fs, path::PathBuf};

fn main() {
    // Window config, application manifest and the exe icon resource.
    tauri_build::build();

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("installer-shell must sit one level below the repository root")
        .to_path_buf();

    let nsis_dir = repo.join("src-tauri/target/release/bundle/nsis");
    println!("cargo:rerun-if-changed={}", nsis_dir.display());

    // The filename carries the version (Brume_0.1.0_x64-setup.exe), so it is
    // discovered rather than hardcoded - otherwise every version bump would
    // silently embed a stale payload or fail to find one at all.
    let payload = fs::read_dir(&nsis_dir).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                    && p.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase().contains("setup"))
                        .unwrap_or(false)
            })
            // Newest wins, so a rebuild is always picked up over a stale artifact.
            .max_by_key(|p| {
                fs::metadata(p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
    });

    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is always set by cargo"))
        .join("payload.exe");

    match payload {
        Some(src) => {
            fs::copy(&src, &dest).expect("failed to stage the NSIS payload for embedding");
            println!("cargo:warning=embedding installer payload: {}", src.display());
        }
        None => panic!(
            "No NSIS installer found in {}.\n\
             The browser must be bundled before the installer shell is built.\n\
             Run `npm run tauri build` first, or use tools/build-installer.ps1 \
             which sequences both stages.",
            nsis_dir.display()
        ),
    }
}
