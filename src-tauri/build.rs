fn main() {
    // The Linux runner is built without the `app` feature, so it does not
    // need Tauri's bundling step (docs/architecture.md, Remote hosts).
    #[cfg(feature = "app")]
    tauri_build::build();
}
