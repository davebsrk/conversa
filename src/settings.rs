//! Ajustes de la app: se guardan en `settings.json` y se editan desde la ventana de Ajustes
//! (`ui/settings.html`), la única que puede llamar a estos comandos (ver `capabilities/`).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt as _;

pub const SETTINGS_WINDOW: &str = "settings";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Al arrancar con el sistema, empezar oculta en la bandeja.
    pub start_hidden: bool,
    /// Cerrar la ventana la manda a la bandeja en lugar de salir.
    pub close_to_tray: bool,
    pub updates: UpdateMode,
    /// Mostrar el texto del mensaje en las notificaciones.
    pub message_preview: bool,
    pub global_shortcut: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_hidden: true,
            close_to_tray: true,
            updates: UpdateMode::Notify,
            message_preview: true,
            global_shortcut: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// Avisar con una notificación y esperar a que el usuario acepte.
    Notify,
    /// Instalar sin preguntar.
    Auto,
    /// No buscar actualizaciones.
    Off,
}

pub struct SettingsState(Mutex<Settings>);

pub fn init(app: &AppHandle) {
    let settings = path(app)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    app.manage(SettingsState(Mutex::new(settings)));
}

pub fn get(app: &AppHandle) -> Settings {
    app.state::<SettingsState>()
        .0
        .lock()
        .expect("ajustes")
        .clone()
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.unminimize();
        return window.set_focus();
    }
    WebviewWindowBuilder::new(
        app,
        SETTINGS_WINDOW,
        WebviewUrl::App("settings.html".into()),
    )
    .title("Ajustes de Conversa")
    .inner_size(480.0, 720.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .center()
    // Mismo perfil que la ventana principal: comparte sus procesos de WebView2 y no gasta más.
    .data_directory(crate::profile_dir(app)?)
    .build()?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    settings: Settings,
    /// El inicio con el sistema lo guarda el propio sistema, no `settings.json`.
    autostart: bool,
    version: String,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> SettingsView {
    SettingsView {
        settings: get(&app),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings, autostart: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    if autostart != autolaunch.is_enabled().unwrap_or(false) {
        let result = if autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        result.map_err(|err| format!("No se pudo cambiar el inicio con el sistema: {err}"))?;
    }
    crate::set_global_shortcut(&app, settings.global_shortcut);

    let path = path(&app).ok_or("No se encontró la carpeta de configuración")?;
    std::fs::create_dir_all(path.parent().expect("tiene carpeta"))
        .and_then(|_| std::fs::write(&path, serde_json::to_vec_pretty(&settings).expect("JSON")))
        .map_err(|err| format!("No se pudieron guardar los ajustes: {err}"))?;
    *app.state::<SettingsState>().0.lock().expect("ajustes") = settings;
    Ok(())
}

#[tauri::command]
pub fn check_updates(app: AppHandle) {
    crate::updates::check_now(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"closeToTray": false}"#).unwrap();
        assert!(!settings.close_to_tray);
        assert!(settings.message_preview);
        assert!(settings.updates == UpdateMode::Notify);
    }

    #[test]
    fn update_mode_uses_lowercase_names() {
        let settings: Settings = serde_json::from_str(r#"{"updates": "auto"}"#).unwrap();
        assert!(settings.updates == UpdateMode::Auto);
    }
}
