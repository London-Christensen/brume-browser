// Cargo build script.
//
// `tauri_build::build()` generates the context that `tauri::generate_context!`
// expands at compile time, and on Windows it also embeds the application
// manifest and icon resources into the executable.
fn main() {
    tauri_build::build()
}
