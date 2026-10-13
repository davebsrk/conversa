fn main() {
    // Declarar los comandos propios los somete a los permisos de `capabilities/`: solo el panel
    // de Ajustes puede llamarlos, nunca la página de WhatsApp Web.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_settings",
            "save_settings",
            "check_updates",
            "install_update",
            "close_settings",
            "pause_shortcuts",
            "resume_shortcuts",
            "add_account",
            "switch_account",
            "remove_account",
        ]),
    ))
    .expect("error en tauri-build");
}
