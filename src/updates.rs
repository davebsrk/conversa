//! Busca versiones nuevas en GitHub Releases. Si hay una, avisa con una notificación y solo se
//! instala cuando el usuario lo pide (botón de la notificación o "Buscar actualizaciones"), salvo
//! que en Ajustes se haya elegido instalarlas automáticamente.

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::settings::{self, UpdateMode};

/// La app pasa días en la bandeja: no basta con comprobar al arrancar.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Comprueba al arrancar y cada 6 horas; según el ajuste avisa (una vez por versión) o instala.
pub fn spawn_periodic_check(app: AppHandle) {
    // Una compilación de desarrollo siempre es "antigua": se actualizaría a la publicada.
    if cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || {
        let mut notified = String::new();
        loop {
            let mode = settings::get(&app).updates;
            if mode != UpdateMode::Off
                && let Ok(Some(update)) = check(&app)
                && update.version != notified
            {
                notified = update.version.clone();
                match mode {
                    UpdateMode::Auto => install(&app, update),
                    _ => announce(&app, update),
                }
            }
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// La versión que encontró "Buscar ahora" en Ajustes, lista para instalar si el usuario quiere.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

/// "Buscar ahora" en Ajustes: devuelve la versión nueva si la hay (y la deja preparada).
pub async fn check_for_panel(app: &AppHandle) -> Result<Option<String>, String> {
    let update = app
        .updater()
        .map_err(|err| err.to_string())?
        .check()
        .await
        .map_err(|err| format!("No se pudo comprobar: {err}"))?;
    let version = update.as_ref().map(|update| update.version.clone());
    *app.state::<PendingUpdate>()
        .0
        .lock()
        .expect("actualización") = update;
    Ok(version)
}

/// "Actualizar ahora" en Ajustes: instala la versión encontrada y reinicia Conversa.
pub async fn install_pending(app: &AppHandle) -> Result<(), String> {
    let update = app
        .state::<PendingUpdate>()
        .0
        .lock()
        .expect("actualización")
        .take()
        .ok_or("Vuelve a buscar actualizaciones.")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|err| format!("No se pudo actualizar: {err}"))?;
    app.restart();
}

fn check(app: &AppHandle) -> tauri_plugin_updater::Result<Option<Update>> {
    tauri::async_runtime::block_on(async { app.updater()?.check().await })
}

fn announce(app: &AppHandle, update: Update) {
    let title = format!("Conversa {} disponible", update.version);
    let body = "Pulsa para actualizar. Se reiniciará en unos segundos.";
    #[cfg(windows)]
    {
        let app = app.clone();
        let result = crate::toast::show_simple(&title, body, Some("Actualizar ahora"), move || {
            install(&app, update.clone())
        });
        if let Err(err) = result {
            eprintln!("no se pudo avisar de la actualización: {err}");
        }
    }
    // ponytail: en Linux solo avisa la consola; "Buscar actualizaciones" en la bandeja sí funciona.
    #[cfg(not(windows))]
    {
        let _ = (app, update);
        eprintln!("{title}: {body}");
    }
}

fn install(app: &AppHandle, update: Update) {
    let result = tauri::async_runtime::block_on(update.download_and_install(|_, _| {}, || {}));
    match result {
        // En Windows el instalador cierra la app y la vuelve a abrir; esto cubre el resto.
        Ok(()) => app.restart(),
        Err(err) => notify("No se pudo actualizar Conversa", &err.to_string()),
    }
}

fn notify(title: &str, body: &str) {
    #[cfg(windows)]
    let _ = crate::toast::show_simple(title, body, None, || {});
    #[cfg(not(windows))]
    eprintln!("{title}: {body}");
}
