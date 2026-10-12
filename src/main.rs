// En release no abrimos una consola junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod links;

use std::path::PathBuf;

use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

use links::Target;

const WHATSAPP_URL: &str = "https://web.whatsapp.com/";
const MAIN_WINDOW: &str = "main";

/// WhatsApp Web rechaza WebKitGTK como "navegador no soportado", así que en Linux nos
/// presentamos como un Chrome de escritorio. En Windows WebView2 ya envía un UA de Edge válido.
#[cfg(target_os = "linux")]
const LINUX_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

fn main() {
    tauri::Builder::default()
        // Debe registrarse el primero: si ya hay una instancia, enfocamos su ventana y salimos.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        // Recuerda tamaño, posición y maximizado. La visibilidad no: siempre arrancamos visibles.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .build(),
        )
        .setup(|app| {
            // Un directorio de datos por cuenta: deja preparada la multicuenta.
            let profile_dir = app.path().app_data_dir()?.join("profiles").join("default");
            let on_nav = app.handle().clone();
            let on_new = app.handle().clone();

            let builder = WebviewWindowBuilder::new(
                app,
                MAIN_WINDOW,
                WebviewUrl::External(WHATSAPP_URL.parse().expect("URL válida")),
            )
            .title("Conversa")
            .inner_size(1100.0, 750.0)
            .min_inner_size(400.0, 500.0)
            .data_directory(profile_dir)
            .on_navigation(move |url| match links::classify(url) {
                Target::Internal => true,
                Target::External => {
                    open_external(&on_nav, url);
                    false
                }
                Target::Blocked => false,
            })
            .on_new_window(move |url, _features| match links::classify(&url) {
                Target::Internal => NewWindowResponse::Allow,
                Target::External => {
                    open_external(&on_new, &url);
                    NewWindowResponse::Deny
                }
                Target::Blocked => NewWindowResponse::Deny,
            })
            .on_download(|webview, event| {
                if let DownloadEvent::Requested { url, destination } = event {
                    // WebView2 ya propone una ruta en Descargas; WebKitGTK puede no hacerlo.
                    if !destination.is_absolute() {
                        if let Some(path) = default_download_path(webview.app_handle(), &url) {
                            *destination = path;
                        }
                    }
                }
                true
            });

            #[cfg(target_os = "linux")]
            let builder = builder.user_agent(LINUX_USER_AGENT);

            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Conversa");
}

fn open_external(app: &AppHandle, url: &Url) {
    if let Err(err) = app.opener().open_url(url.as_str(), None::<&str>) {
        eprintln!("no se pudo abrir {url}: {err}");
    }
}

/// Ruta en la carpeta Descargas sin sobrescribir archivos existentes: `foto.jpg`, `foto (1).jpg`...
fn default_download_path(app: &AppHandle, url: &Url) -> Option<PathBuf> {
    let dir = app.path().download_dir().ok()?;
    let name = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| !name.is_empty())
        .unwrap_or("descarga");
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (0..)
        .map(|n| match n {
            0 => dir.join(format!("{stem}{ext}")),
            n => dir.join(format!("{stem} ({n}){ext}")),
        })
        .find(|path| !path.exists())
}
