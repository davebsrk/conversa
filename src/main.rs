// En release no abrimos una consola junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
        .setup(|app| {
            // Un directorio de datos por cuenta: deja preparada la multicuenta.
            let profile_dir = app.path().app_data_dir()?.join("profiles").join("default");

            let builder = WebviewWindowBuilder::new(
                app,
                MAIN_WINDOW,
                WebviewUrl::External(WHATSAPP_URL.parse().expect("URL válida")),
            )
            .title("Conversa")
            .inner_size(1100.0, 750.0)
            .min_inner_size(400.0, 500.0)
            .data_directory(profile_dir);

            #[cfg(target_os = "linux")]
            let builder = builder.user_agent(LINUX_USER_AGENT);

            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Conversa");
}
