//! Ajustes de la app: se guardan en `settings.json` y se editan desde el panel de Ajustes
//! (`ui/settings.html`), la única webview que puede llamar a estos comandos (ver `capabilities/`).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Webview, WebviewBuilder, WebviewUrl, Window,
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
    /// Activa el atajo global para mostrar u ocultar la ventana (`toggle_shortcut`).
    pub global_shortcut: bool,
    /// Combinación con los nombres de tecla del navegador, p. ej. `Ctrl+Alt+KeyW`.
    pub toggle_shortcut: String,
    /// Atajo global para activar o desactivar el modo privacidad; vacío = sin atajo.
    pub privacy_shortcut: String,
    /// Zoom de WhatsApp Web (1.0 = 100 %), para quien necesita letra más grande.
    pub zoom: f64,
    /// Desenfocar la lista de chats y los mensajes hasta pasar el ratón por encima.
    pub privacy_blur: bool,
    pub list_width: ListWidth,
    /// Ocultar la barra de filtros (Todos, No leídos, Favoritos...) de la lista de chats.
    pub hide_filters: bool,
    pub hide_status: bool,
    pub hide_channels: bool,
    pub hide_communities: bool,
    /// Cuentas de WhatsApp (ver `accounts`). Siempre hay al menos una.
    pub accounts: Vec<Account>,
    pub active_account: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
}

/// Ancho de la columna de la lista de chats.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ListWidth {
    /// El de WhatsApp.
    Auto,
    Narrow,
    Wide,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_hidden: true,
            close_to_tray: true,
            updates: UpdateMode::Notify,
            message_preview: true,
            global_shortcut: true,
            toggle_shortcut: "Ctrl+Alt+KeyW".into(),
            privacy_shortcut: String::new(),
            zoom: 1.0,
            privacy_blur: false,
            list_width: ListWidth::Auto,
            hide_filters: false,
            hide_status: false,
            hide_channels: false,
            hide_communities: false,
            accounts: vec![Account {
                id: "default".into(),
                name: "Cuenta 1".into(),
            }],
            active_account: "default".into(),
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
    let mut settings: Settings = path(app)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if settings.accounts.is_empty() {
        settings.accounts = Settings::default().accounts;
    }
    if !settings
        .accounts
        .iter()
        .any(|a| a.id == settings.active_account)
    {
        settings.active_account = settings.accounts[0].id.clone();
    }
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
        // Abierto o cerrándose: vuelve a su sitio (la animación nueva sustituye a la de cierre).
        let from = panel.position().map(|position| position.x).unwrap_or(0);
        let generation = PANEL_ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || slide(&panel, from, 0, false, generation));
        return Ok(());
    }
    let (_, size) = panel_bounds(&window)?;
    let builder = WebviewBuilder::new(SETTINGS_PANEL, WebviewUrl::App("settings.html".into()))
        // Mismo perfil que WhatsApp: comparte sus procesos de WebView2 y no gasta más.
        .data_directory(crate::accounts::profile_dir(app, &get(app).active_account)?);
    // Entra deslizándose desde la izquierda.
    let hidden = -(size.width as i32);
    let panel = window.add_child(builder, PhysicalPosition::new(hidden, 0), size)?;
    panel.set_focus()?;
    let generation = PANEL_ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || slide(&panel, hidden, 0, false, generation));
    Ok(())
}

/// Cada animación del panel tiene un número; si empieza otra, la anterior se detiene.
static PANEL_ANIMATION: AtomicU64 = AtomicU64::new(0);

/// Desliza el panel en horizontal de `from` a `to` (en ~160 ms, frenando al final) y, si se pide,
/// lo cierra al terminar. Cada webview es una ventana nativa: se mueve la ventana entera.
fn slide(panel: &Webview, from: i32, to: i32, close: bool, generation: u64) {
    const STEPS: i32 = 10;
    for step in 1..=STEPS {
        if PANEL_ANIMATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let t = f64::from(step) / f64::from(STEPS);
        let eased = 1.0 - (1.0 - t).powi(3);
        let x = from + (f64::from(to - from) * eased).round() as i32;
        if panel.set_position(PhysicalPosition::new(x, 0)).is_err() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    if close && PANEL_ANIMATION.load(Ordering::SeqCst) == generation {
        let _ = panel.close();
    }
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
    let Some(panel) = app.get_webview(SETTINGS_PANEL) else {
        return;
    };
    // Sale deslizándose hacia la izquierda y se cierra.
    let width = panel.size().map(|size| size.width as i32).unwrap_or(0);
    let from = panel.position().map(|position| position.x).unwrap_or(0);
    let generation = PANEL_ANIMATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || slide(&panel, from, -width, true, generation));
}

/// Lo que Conversa cambia en el aspecto de WhatsApp Web: CSS y elementos que ocultar (botones de
/// la barra lateral por el título de su icono, y `filters` para la barra de filtros).
#[derive(Serialize)]
pub struct Appearance {
    css: String,
    hide: Vec<&'static str>,
    /// Botones de cuenta de la barra lateral (solo si hay más de una).
    accounts: Vec<AccountButton>,
}

#[derive(Serialize)]
struct AccountButton {
    id: String,
    name: String,
    unread: u32,
    /// La cuenta de esta webview.
    current: bool,
}

/// Aspecto de la webview de la cuenta `current` (`unread`: chats sin leer de cada cuenta).
pub fn appearance(settings: &Settings, current: &str, unread: impl Fn(&str) -> u32) -> Appearance {
    let hide = [
        (settings.hide_filters, "filters"),
        (settings.hide_status, "wds-ic-status"),
        (settings.hide_channels, "wds-ic-channels"),
        (settings.hide_communities, "wds-ic-communities"),
    ]
    .into_iter()
    .filter_map(|(on, item)| on.then_some(item))
    .collect();
    let accounts = if settings.accounts.len() > 1 {
        settings
            .accounts
            .iter()
            .map(|account| AccountButton {
                id: account.id.clone(),
                name: account.name.clone(),
                unread: unread(&account.id),
                current: account.id == current,
            })
            .collect()
    } else {
        Vec::new()
    };
    Appearance {
        css: appearance_css(settings),
        hide,
        accounts,
    }
}

fn appearance_css(settings: &Settings) -> String {
    let mut css = String::new();
    // La columna de la lista y el hueco equivalente de la capa del chat (los marca el script
    // inyectado con data-conversa-list; WhatsApp les da un 30-40 %).
    let width = match settings.list_width {
        ListWidth::Auto => None,
        ListWidth::Narrow => Some("28%"),
        ListWidth::Wide => Some("50%"),
    };
    if let Some(width) = width {
        css.push_str(&format!(
            "[data-conversa-list] {{ flex: 0 0 {width} !important; max-width: {width} !important; min-width: 280px; }}\n"
        ));
    }
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

/// Aplica el aspecto de los ajustes a WhatsApp Web. También lo guarda en su `localStorage`, de
/// donde lo lee el script inicial al recargar: así se aplica desde el primer instante, sin parpadeo.
pub fn apply_appearance(app: &AppHandle, settings: &Settings) {
    for webview in crate::accounts::all(app) {
        apply_appearance_to(&webview, settings);
    }
}

pub fn apply_appearance_to(webview: &Webview, settings: &Settings) {
    let Some(current) = crate::accounts::id_of(webview.label()) else {
        return;
    };
    let app = webview.app_handle();
    let appearance = appearance(settings, current, |id| crate::accounts::unread(app, id));
    let appearance = serde_json::to_string(&appearance).expect("JSON");
    let _ = webview.eval(format!("window.conversaApplyAppearance({appearance})"));
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
pub fn save_settings(
    app: AppHandle,
    mut settings: Settings,
    autostart: bool,
) -> Result<(), String> {
    // Las cuentas se añaden y quitan con sus comandos; aquí solo cambian los nombres.
    let current = get(&app);
    settings.accounts = current
        .accounts
        .iter()
        .map(|account| {
            let name = settings
                .accounts
                .iter()
                .find(|a| a.id == account.id)
                .map(|a| a.name.trim().to_string())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| account.name.clone());
            Account {
                id: account.id.clone(),
                name,
            }
        })
        .collect();
    settings.active_account = current.active_account;
    // Primero los atajos: si alguno no se puede usar, no se guarda nada y vuelven los de antes.
    if let Err(err) = crate::set_shortcuts(&app, &settings) {
        let _ = crate::set_shortcuts(&app, &get(&app));
        return Err(err);
    }
    let autolaunch = app.autolaunch();
    if autostart != autolaunch.is_enabled().unwrap_or(false) {
        let result = if autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        result.map_err(|err| format!("No se pudo cambiar el inicio con el sistema: {err}"))?;
    }
    store(&app, settings)
}

/// Guarda los ajustes en `settings.json` y aplica los que afectan a WhatsApp (zoom, aspecto).
pub fn store(app: &AppHandle, settings: Settings) -> Result<(), String> {
    for webview in crate::accounts::all(app) {
        let _ = webview.set_zoom(settings.zoom.clamp(0.5, 3.0));
    }
    apply_appearance(app, &settings);

    let path = path(app).ok_or("No se encontró la carpeta de configuración")?;
    std::fs::create_dir_all(path.parent().expect("tiene carpeta"))
        .and_then(|_| std::fs::write(&path, serde_json::to_vec_pretty(&settings).expect("JSON")))
        .map_err(|err| format!("No se pudieron guardar los ajustes: {err}"))?;
    *app.state::<SettingsState>().0.lock().expect("ajustes") = settings;
    Ok(())
}

/// Atajo de privacidad: invierte el modo y refresca el panel de Ajustes si está abierto.
pub fn toggle_privacy(app: &AppHandle) {
    let mut settings = get(app);
    settings.privacy_blur = !settings.privacy_blur;
    if let Err(err) = store(app, settings) {
        eprintln!("{err}");
    }
    if let Some(panel) = app.get_webview(SETTINGS_PANEL) {
        let _ = panel.eval("load()");
    }
}

/// Mientras el panel graba una combinación nueva: sin atajos, para que la actual no salte.
#[tauri::command]
pub fn pause_shortcuts(app: AppHandle) {
    crate::pause_shortcuts(&app);
}

/// Al cancelar la grabación: vuelven los atajos guardados.
#[tauri::command]
pub fn resume_shortcuts(app: AppHandle) -> Result<(), String> {
    crate::set_shortcuts(&app, &get(&app))
}

/// Cuentas: el panel se cierra para dejar ver la cuenta (la nueva muestra su QR para vincularla).
/// Son `async` para no ejecutarse en el hilo principal: crear una webview espera a ese hilo, y
/// desde él mismo se quedaría bloqueada.
#[tauri::command]
pub async fn add_account(app: AppHandle) -> Result<(), String> {
    close_settings(app.clone());
    crate::accounts::add(&app)?;
    apply_appearance(&app, &get(&app));
    Ok(())
}

#[tauri::command]
pub async fn switch_account(app: AppHandle, id: String) {
    close_settings(app.clone());
    crate::accounts::switch(&app, &id);
    apply_appearance(&app, &get(&app));
}

#[tauri::command]
pub async fn remove_account(app: AppHandle, id: String) -> Result<(), String> {
    crate::accounts::remove(&app, &id)?;
    apply_appearance(&app, &get(&app));
    Ok(())
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<Option<String>, String> {
    crate::updates::check_for_panel(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    crate::updates::install_pending(&app).await
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
        assert!(settings.list_width == ListWidth::Auto);
        assert_eq!(settings.toggle_shortcut, "Ctrl+Alt+KeyW");
        assert!(settings.privacy_shortcut.is_empty());
        assert_eq!(settings.accounts.len(), 1);
        assert_eq!(settings.active_account, "default");
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
    fn list_width_and_hidden_items() {
        let mut settings = Settings::default();
        let plain = appearance(&settings, "default", |_| 0);
        assert!(plain.css.is_empty() && plain.hide.is_empty());
        settings.list_width = ListWidth::Wide;
        settings.hide_filters = true;
        settings.hide_channels = true;
        let custom = appearance(&settings, "default", |_| 0);
        assert!(
            custom
                .css
                .contains("[data-conversa-list] { flex: 0 0 50% !important")
        );
        assert_eq!(custom.hide, ["filters", "wds-ic-channels"]);
        let parsed: Settings = serde_json::from_str(r#"{"listWidth": "narrow"}"#).unwrap();
        assert!(parsed.list_width == ListWidth::Narrow);
    }

    #[test]
    fn account_buttons_only_with_several_accounts() {
        let mut settings = Settings::default();
        assert!(appearance(&settings, "default", |_| 0).accounts.is_empty());
        settings.accounts.push(Account {
            id: "a1".into(),
            name: "Trabajo".into(),
        });
        let buttons =
            appearance(&settings, "a1", |id| if id == "default" { 3 } else { 0 }).accounts;
        assert_eq!(buttons.len(), 2);
        assert!(!buttons[0].current && buttons[0].unread == 3);
        assert!(buttons[1].current && buttons[1].name == "Trabajo");
    }

    #[test]
    fn update_mode_uses_lowercase_names() {
        let settings: Settings = serde_json::from_str(r#"{"updates": "auto"}"#).unwrap();
        assert!(settings.updates == UpdateMode::Auto);
    }
}
