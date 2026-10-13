//! Ajustes de la app: se guardan en `settings.json` y se editan desde el panel de Ajustes
//! (`ui/settings.html`), la única webview que puede llamar a estos comandos (ver `capabilities/`).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewBuilder, WebviewUrl, Window,
};
use tauri_plugin_autostart::ManagerExt as _;

pub const SETTINGS_PANEL: &str = "settings";
/// Ancho del panel, como la columna de la lista de chats de WhatsApp.
const PANEL_WIDTH: f64 = 420.0;

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
    /// Zoom de WhatsApp Web (1.0 = 100 %), para quien necesita letra más grande.
    pub zoom: f64,
    /// Desenfocar la lista de chats y los mensajes hasta pasar el ratón por encima.
    pub privacy_blur: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_hidden: true,
            close_to_tray: true,
            updates: UpdateMode::Notify,
            message_preview: true,
            global_shortcut: true,
            zoom: 1.0,
            privacy_blur: false,
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

/// Abre los Ajustes como un panel a la izquierda de la ventana principal, encima de la lista de
/// chats (como el panel de ajustes de WhatsApp). Es una webview aparte, con nuestra página: así
/// WhatsApp Web sigue sin acceso a estos comandos.
pub fn open_panel(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_window(crate::MAIN_WINDOW) else {
        return Ok(());
    };
    crate::show_window(app);
    if let Some(panel) = app.get_webview(SETTINGS_PANEL) {
        return panel.set_focus();
    }
    let (position, size) = panel_bounds(&window)?;
    let builder = WebviewBuilder::new(SETTINGS_PANEL, WebviewUrl::App("settings.html".into()))
        // Mismo perfil que WhatsApp: comparte sus procesos de WebView2 y no gasta más.
        .data_directory(crate::profile_dir(app)?);
    window.add_child(builder, position, size)?.set_focus()
}

/// El panel ocupa toda la altura y hasta `PANEL_WIDTH` de ancho; se llama al redimensionar.
pub fn resize_panel(window: &Window) {
    if let Some(panel) = window.app_handle().get_webview(SETTINGS_PANEL)
        && let Ok((position, size)) = panel_bounds(window)
    {
        let _ = panel.set_position(position);
        let _ = panel.set_size(size);
    }
}

fn panel_bounds(window: &Window) -> tauri::Result<(PhysicalPosition<i32>, PhysicalSize<u32>)> {
    let size = window.inner_size()?;
    let width = ((PANEL_WIDTH * window.scale_factor()?) as u32).min(size.width);
    Ok((
        PhysicalPosition::new(0, 0),
        PhysicalSize::new(width, size.height),
    ))
}

#[tauri::command]
pub fn close_settings(app: AppHandle) {
    if let Some(panel) = app.get_webview(SETTINGS_PANEL) {
        let _ = panel.close();
    }
}

/// CSS que Conversa añade a WhatsApp Web según los ajustes.
pub fn appearance_css(settings: &Settings) -> String {
    let mut css = String::new();
    if settings.privacy_blur {
        // Lista de chats, mensajes y cabecera del chat; se ven nítidos al pasar el ratón.
        css.push_str(
            "#pane-side [role=\"row\"], #main [role=\"row\"], #main header \
             { filter: blur(7px); transition: filter .15s ease; }\n\
             #pane-side [role=\"row\"]:hover, #main [role=\"row\"]:hover, #main header:hover \
             { filter: none; }\n",
        );
    }
    css
}

/// Aplica el CSS de los ajustes a WhatsApp Web. También lo guarda en su `localStorage`, de donde
/// lo lee el script inicial al recargar: así se aplica desde el primer instante, sin parpadeo.
pub fn apply_appearance(app: &AppHandle, settings: &Settings) {
    if let Some(webview) = app.get_webview(crate::MAIN_WINDOW) {
        let css = serde_json::to_string(&appearance_css(settings)).expect("JSON");
        let _ = webview.eval(format!("window.conversaApplyCss({css})"));
    }
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
    if let Some(webview) = app.get_webview(crate::MAIN_WINDOW) {
        let _ = webview.set_zoom(settings.zoom.clamp(0.5, 3.0));
    }
    apply_appearance(&app, &settings);

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
        assert_eq!(settings.zoom, 1.0);
        assert!(!settings.privacy_blur);
    }

    #[test]
    fn privacy_blur_adds_css_only_when_enabled() {
        let mut settings = Settings::default();
        assert_eq!(appearance_css(&settings), "");
        settings.privacy_blur = true;
        let css = appearance_css(&settings);
        assert!(css.contains("#pane-side [role=\"row\"]") && css.contains("blur(7px)"));
        assert!(css.contains(":hover"));
    }

    #[test]
    fn update_mode_uses_lowercase_names() {
        let settings: Settings = serde_json::from_str(r#"{"updates": "auto"}"#).unwrap();
        assert!(settings.updates == UpdateMode::Auto);
    }
}
