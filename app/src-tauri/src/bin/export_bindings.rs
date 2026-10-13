//! `cargo run -p autoscene-app --bin export-bindings` regenerates
//! `app/src/bindings.ts` without launching the app. (A bin rather than a test:
//! on Windows only bins get Tauri's embedded app manifest.)

fn main() {
    autoscene_app::export_bindings(&autoscene_app::specta_builder());
    println!("wrote {}", autoscene_app::BINDINGS_PATH);
}
