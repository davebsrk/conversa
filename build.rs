fn main() {
    // Declarar los comandos propios los somete a los permisos de `capabilities/`: solo el panel
    // de Ajustes puede llamarlos, nunca la página de WhatsApp Web.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_settings",
            "save_settings",
            "check_updates",
            "close_settings",
            "pause_shortcuts",
            "resume_shortcuts",
        ]),
    ))
    .expect("error en tauri-build");
}
