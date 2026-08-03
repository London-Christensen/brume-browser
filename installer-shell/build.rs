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

    // Pinned to this crate's own version, not "whichever file is newest".
    //
    // new-release.ps1 bumps the shell and the browser together, so the version
    // here is the version the payload must carry. That directory is never
    // cleaned and holds every build ever made, so picking by timestamp meant a
    // half-finished or hand-run build could leave the shell wrapping an
    // installer for a completely different version, with nothing to notice it.
    let version = env::var("CARGO_PKG_VERSION").expect("cargo always sets CARGO_PKG_VERSION");
    let marker = format!("_{version}_");

    let candidates: Vec<PathBuf> = fs::read_dir(&nsis_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                && p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase().contains("setup"))
                    .unwrap_or(false)
        })
        .collect();

    let payload = candidates
        .iter()
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().contains(&marker))
                .unwrap_or(false)
        })
        // Still newest-wins, but only among artifacts for this version.
        .max_by_key(|p| {
            fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .cloned();

    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is always set by cargo"))
        .join("payload.exe");

    match payload {
        Some(src) => {
            // Watch the file itself, not only the directory it sits in.
            //
            // A directory's mtime moves when an entry is added or removed, but
            // not when an existing file is overwritten. Rebuilding without
            // bumping the version overwrites the installer in place, so the
            // directory looked untouched, this script did not re-run, and the
            // stale payload.exe already in OUT_DIR was the one embedded. The
            // shipped Brume-Setup.exe then quietly installed the previous build.
            println!("cargo:rerun-if-changed={}", src.display());

            fs::copy(&src, &dest).expect("failed to stage the NSIS payload for embedding");
            println!("cargo:warning=embedding installer payload: {}", src.display());
        }
        None => {
            let seen = if candidates.is_empty() {
                "        (none)".to_string()
            } else {
                candidates
                    .iter()
                    .map(|p| format!("        {}", p.file_name().unwrap_or_default().to_string_lossy()))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            panic!(
                "No NSIS installer for version {version} found in {}.\n\
                 The browser must be bundled at a matching version before the \
                 installer shell is built.\n\
                 Run `npm run tauri build` first, or use tools/build-installer.ps1 \
                 which sequences both stages.\n\
                 Present in that directory:\n{seen}",
                nsis_dir.display()
            )
        }
    }
}
