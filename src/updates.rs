//! Busca versiones nuevas en GitHub Releases. Si hay una, avisa con una notificación y solo se
//! instala cuando el usuario lo pide (botón de la notificación o "Buscar actualizaciones"), salvo
//! que en Ajustes se haya elegido instalarlas automáticamente.

use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::settings::{self, UpdateMode};

/// La app pasa días en la bandeja: no basta con comprobar al arrancar.
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Comprueba al arrancar y cada 6 horas; según el ajuste avisa (una vez por versión) o instala.
pub fn spawn_periodic_check(app: AppHandle) {
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

/// Desde el menú de la bandeja: si hay versión nueva la instala directamente (ya la ha pedido).
pub fn check_now(app: AppHandle) {
    std::thread::spawn(move || match check(&app) {
        Ok(Some(update)) => install(&app, update),
        Ok(None) => notify(
            "Conversa está actualizada",
            &format!("Tienes la última versión ({}).", app.package_info().version),
        ),
        Err(err) => notify("No se pudo buscar actualizaciones", &err.to_string()),
    });
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
