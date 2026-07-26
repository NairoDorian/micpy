// Build script for the micpy Tauri crate.
// Invokes `tauri_build` to embed the frontend build artifacts into the Rust source.
fn main() {
    tauri_build::build()
}
