//! Varias cuentas de WhatsApp: una webview por cuenta dentro de la ventana principal, cada una con
//! su carpeta de datos (su propia sesión). Solo se ve la activa; las demás siguen conectadas, para
//! recibir mensajes y notificaciones, pero en el modo de bajo consumo de memoria.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager, PhysicalPosition, Url, Webview, Window};

use crate::settings::{self, Account};

/// Las webviews de WhatsApp se llaman `wa-<id de la cuenta>`.
const PREFIX: &str = "wa-";

pub fn label(id: &str) -> String {
    format!("{PREFIX}{id}")
}

pub fn id_of(label: &str) -> Option<&str> {
    label.strip_prefix(PREFIX)
}

/// La carpeta de datos (sesión de WhatsApp) de una cuenta. La primera cuenta es `default`, la
/// carpeta que ya usaba Conversa antes de tener varias cuentas.
pub fn profile_dir(app: &AppHandle, id: &str) -> tauri::Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("profiles").join(id))
}

pub fn webview(app: &AppHandle, id: &str) -> Option<Webview> {
    app.get_webview(&label(id))
}

pub fn active(app: &AppHandle) -> Option<Webview> {
    webview(app, &settings::get(app).active_account)
}

pub fn all(app: &AppHandle) -> Vec<Webview> {
    app.webviews()
        .into_values()
        .filter(|webview| id_of(webview.label()).is_some())
        .collect()
}

/// Chats sin leer de cada cuenta (lo que WhatsApp Web pone en su título).
#[derive(Default)]
pub struct Unread(Mutex<HashMap<String, u32>>);

/// Guarda los no leídos de una cuenta y devuelve el total de todas.
pub fn set_unread(app: &AppHandle, id: &str, count: u32) -> u32 {
    let state = app.state::<Unread>();
    let mut counts = state.0.lock().expect("no leídos");
    counts.insert(id.to_string(), count);
    counts.values().sum()
}

pub fn unread(app: &AppHandle, id: &str) -> u32 {
    let state = app.state::<Unread>();
    let counts = state.0.lock().expect("no leídos");
    counts.get(id).copied().unwrap_or(0)
}

/// Crea la webview de una cuenta ocupando toda la ventana; oculta salvo que sea la activa.
pub fn create(
    app: &AppHandle,
    window: &Window,
    account: &Account,
    url: Url,
    visible: bool,
) -> tauri::Result<Webview> {
    let builder = crate::whatsapp_webview(app, &account.id, url)?.auto_resize();
    let webview = window.add_child(builder, PhysicalPosition::new(0, 0), window.inner_size()?)?;
    if !visible {
        webview.hide()?;
    }
    let _ = webview.set_zoom(settings::get(app).zoom);
    // En Linux WebKitGTK ya envía las notificaciones web al sistema.
    #[cfg(windows)]
    crate::toast::attach(&webview);
    Ok(webview)
}

/// Muestra la cuenta `id` y oculta las demás.
pub fn switch(app: &AppHandle, id: &str) {
    let mut settings = settings::get(app);
    if !settings.accounts.iter().any(|account| account.id == id) {
        return;
    }
    if settings.active_account != id {
        settings.active_account = id.to_string();
        if let Err(err) = settings::store(app, settings) {
            eprintln!("{err}");
        }
    }
    for webview in all(app) {
        if webview.label() == label(id) {
            let _ = webview.show();
            let _ = webview.set_focus();
        } else {
            let _ = webview.hide();
        }
    }
    crate::refresh_memory(app);
}

/// Añade una cuenta nueva (su sesión empieza vacía: WhatsApp pedirá escanear el QR) y la muestra.
pub fn add(app: &AppHandle) -> Result<String, String> {
    let window = app
        .get_window(crate::MAIN_WINDOW)
        .ok_or("No se encontró la ventana principal")?;
    let mut settings = settings::get(app);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let account = Account {
        id: format!("a{millis}"),
        name: format!("Cuenta {}", settings.accounts.len() + 1),
    };
    settings.accounts.push(account.clone());
    settings::store(app, settings)?;
    let url = crate::WHATSAPP_URL.parse().expect("URL válida");
    create(app, &window, &account, url, false)
        .map_err(|err| format!("No se pudo crear la cuenta: {err}"))?;
    switch(app, &account.id);
    Ok(account.id)
}

/// Quita una cuenta de Conversa: cierra su webview y borra su sesión de este equipo (la cuenta de
/// WhatsApp en sí no se toca). Siempre queda al menos una.
pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut settings = settings::get(app);
    if settings.accounts.len() <= 1 {
        return Err("Tiene que quedar al menos una cuenta.".into());
    }
    settings.accounts.retain(|account| account.id != id);
    if settings.active_account == id {
        settings.active_account = settings.accounts[0].id.clone();
    }
    let next = settings.active_account.clone();
    settings::store(app, settings)?;
    switch(app, &next);
    if let Some(webview) = webview(app, id) {
        let _ = webview.close();
    }
    set_unread(app, id, 0);
    app.state::<Unread>()
        .0
        .lock()
        .expect("no leídos")
        .remove(id);
    crate::refresh_unread(app);
    // WebView2 suelta los archivos al cerrar su proceso, un momento después.
    if let Ok(dir) = profile_dir(app, id) {
        std::thread::spawn(move || {
            for _ in 0..40 {
                if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }
    Ok(())
}

/// Al arrancar: borra las carpetas de cuentas que se quitaron pero no se pudieron borrar entonces.
pub fn clean_removed_profiles(app: &AppHandle) {
    let settings = settings::get(app);
    let Ok(dir) = app.path().app_data_dir().map(|dir| dir.join("profiles")) else {
        return;
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !settings.accounts.iter().any(|account| account.id == name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_round_trip() {
        assert_eq!(label("default"), "wa-default");
        assert_eq!(id_of("wa-a123"), Some("a123"));
        assert_eq!(id_of("settings"), None);
    }
}
