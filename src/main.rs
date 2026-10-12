// En release no abrimos una consola junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod links;

use std::path::PathBuf;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse, PermissionKind, PermissionResponse};
use tauri::{
    AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

use links::Target;

const WHATSAPP_URL: &str = "https://web.whatsapp.com/";
const MAIN_WINDOW: &str = "main";
const TRAY: &str = "main";
const TOGGLE_SHORTCUT: &str = "CommandOrControl+Alt+W";
/// Argumento con el que se lanza al iniciar sesión en el sistema: arranca oculta en la bandeja.
const MINIMIZED_ARG: &str = "--minimized";
const TRAY_UNREAD_ICON: &[u8] = include_bytes!("../icons/tray-unread.png");
/// Oculta el botón "Obtener WhatsApp para Windows" del pie de la lista de chats. Si WhatsApp
/// cambia su HTML, el selector deja de coincidir y simplemente vuelve a verse.
const HIDE_ELEMENTS_SCRIPT: &str = r#"
document.addEventListener("DOMContentLoaded", () => {
  const style = document.createElement("style");
  style.textContent = `#side > [role="button"]:has([data-icon="wa-square-icon"]) { display: none !important; }`;
  document.head.append(style);
});
"#;

/// WhatsApp Web rechaza WebKitGTK como "navegador no soportado", así que en Linux nos
/// presentamos como un Chrome de escritorio. En Windows WebView2 ya envía un UA de Edge válido.
#[cfg(target_os = "linux")]
const LINUX_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

fn main() {
    tauri::Builder::default()
        // Debe registrarse el primero: si ya hay una instancia, mostramos su ventana y salimos.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                show_window(&window);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        // Recuerda tamaño, posición y maximizado. La visibilidad la decidimos nosotros.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() - StateFlags::VISIBLE)
                .build(),
        )
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args([MINIMIZED_ARG])
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_window_event(|window, event| match event {
            // Cerrar la ventana la manda a la bandeja; se sale desde el menú de la bandeja.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if let Some(window) = window.get_webview_window(MAIN_WINDOW) {
                    hide_window(&window);
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(window) = window.get_webview_window(MAIN_WINDOW) {
                    set_background(&window, window.is_minimized().unwrap_or(false));
                }
            }
            _ => {}
        })
        .setup(|app| {
            let start_hidden = std::env::args().any(|arg| arg == MINIMIZED_ARG);
            let window = build_main_window(app.handle(), !start_hidden)?;
            if start_hidden {
                set_background(&window, true);
            }
            build_tray(app.handle())?;

            if let Err(err) =
                app.global_shortcut()
                    .on_shortcut(TOGGLE_SHORTCUT, |app, _shortcut, event| {
                        if event.state == ShortcutState::Pressed {
                            toggle_window(app);
                        }
                    })
            {
                // Otra app puede tener ya el atajo: no es motivo para no arrancar.
                eprintln!("no se pudo registrar el atajo {TOGGLE_SHORTCUT}: {err}");
            }

            #[cfg(not(debug_assertions))]
            tauri::async_runtime::spawn(update(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Conversa");
}

fn build_main_window(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    // Un directorio de datos por cuenta: deja preparada la multicuenta.
    let profile_dir = app.path().app_data_dir()?.join("profiles").join("default");
    let on_nav = app.clone();
    let on_new = app.clone();

    let builder = WebviewWindowBuilder::new(
        app,
        MAIN_WINDOW,
        WebviewUrl::External(WHATSAPP_URL.parse().expect("URL válida")),
    )
    .title("Conversa")
    .inner_size(1100.0, 750.0)
    .min_inner_size(400.0, 500.0)
    .visible(visible)
    .data_directory(profile_dir)
    .initialization_script(HIDE_ELEMENTS_SCRIPT)
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
            if !destination.is_absolute()
                && let Some(path) = default_download_path(webview.app_handle(), &url)
            {
                *destination = path;
            }
        }
        true
    })
    // La navegación está limitada a web.whatsapp.com, así que solo WhatsApp Web puede pedirlos.
    // Las notificaciones las muestra la propia webview con el sistema de notificaciones nativo.
    .on_permission_request(|_webview, kind| match kind {
        PermissionKind::Notifications
        | PermissionKind::Microphone
        | PermissionKind::Camera
        | PermissionKind::ClipboardRead => PermissionResponse::Allow,
        _ => PermissionResponse::Default,
    })
    .on_document_title_changed(|window, title| set_unread(&window, unread_count(&title)));

    #[cfg(target_os = "linux")]
    let builder = builder.user_agent(LINUX_USER_AGENT);

    builder.build()
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Mostrar / ocultar", true, None::<&str>)?,
            &CheckMenuItem::with_id(
                app,
                "autostart",
                "Iniciar con el sistema",
                true,
                autostart_enabled,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY)
        .icon(
            app.default_window_icon()
                .expect("icono en tauri.conf.json")
                .clone(),
        )
        .tooltip("Conversa")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_window(app),
            "autostart" => {
                let autolaunch = app.autolaunch();
                let result = if autolaunch.is_enabled().unwrap_or(false) {
                    autolaunch.disable()
                } else {
                    autolaunch.enable()
                };
                if let Err(err) = result {
                    eprintln!("no se pudo cambiar el inicio automático: {err}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    set_background(window, false);
}

fn hide_window(window: &WebviewWindow) {
    let _ = window.hide();
    set_background(window, true);
}

fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let in_front = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if in_front {
        hide_window(&window);
    } else {
        show_window(&window);
    }
}

/// Modo híbrido: oculta o minimizada, pedimos a WebView2 que libere toda la memoria posible,
/// pero sin suspender la página, para que sigan llegando mensajes y notificaciones al momento.
fn set_background(window: &WebviewWindow, background: bool) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
        };
        use windows::core::Interface;

        let level = if background {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        let _ = window.with_webview(move |webview| unsafe {
            if let Ok(core) = webview.controller().CoreWebView2()
                && let Ok(core) = core.cast::<ICoreWebView2_19>()
            {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        });
    }
    // ponytail: WebKitGTK no tiene un equivalente directo; en Linux no hacemos nada de momento.
    #[cfg(not(windows))]
    let _ = (window, background);
}

/// WhatsApp Web pone los no leídos en el título: "(3) WhatsApp".
fn unread_count(title: &str) -> u32 {
    title
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .and_then(|(count, _)| count.parse().ok())
        .unwrap_or(0)
}

fn set_unread(window: &WebviewWindow, count: u32) {
    let label = match count {
        0 => "Conversa".to_string(),
        n => format!("({n}) Conversa"),
    };
    let _ = window.set_title(&label);

    if let Some(tray) = window.app_handle().tray_by_id(TRAY) {
        let icon = match count {
            0 => window.app_handle().default_window_icon().cloned(),
            _ => Image::from_bytes(TRAY_UNREAD_ICON).ok(),
        };
        let _ = tray.set_icon(icon);
        let _ = tray.set_tooltip(Some(&label));
    }

    #[cfg(windows)]
    let _ = window.set_overlay_icon((count > 0).then(red_dot));
    #[cfg(not(windows))]
    let _ = window.set_badge_count((count > 0).then_some(i64::from(count)));
}

/// Punto rojo de 16×16 para el icono superpuesto de la barra de tareas de Windows.
#[cfg(windows)]
fn red_dot() -> Image<'static> {
    const SIZE: u32 = 16;
    let rgba = (0..SIZE * SIZE)
        .flat_map(|i| {
            let (x, y) = ((i % SIZE) as f32 - 7.5, (i / SIZE) as f32 - 7.5);
            let alpha = if x * x + y * y <= 56.0 { 255 } else { 0 };
            [0xe5, 0x38, 0x3b, alpha]
        })
        .collect();
    Image::new_owned(rgba, SIZE, SIZE)
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

/// Al arrancar busca una versión nueva en GitHub Releases y, si la hay, la instala y reinicia.
#[cfg(not(debug_assertions))]
async fn update(app: AppHandle) {
    use tauri_plugin_updater::UpdaterExt;

    let result = async {
        if let Some(update) = app.updater()?.check().await? {
            update.download_and_install(|_, _| {}, || {}).await?;
            app.restart();
        }
        Ok::<_, tauri_plugin_updater::Error>(())
    }
    .await;
    if let Err(err) = result {
        eprintln!("no se pudo comprobar si hay actualizaciones: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::unread_count;

    #[test]
    fn reads_unread_count_from_title() {
        assert_eq!(unread_count("(3) WhatsApp"), 3);
        assert_eq!(unread_count("(120) WhatsApp Business"), 120);
        assert_eq!(unread_count("WhatsApp"), 0);
        assert_eq!(unread_count("(x) WhatsApp"), 0);
        assert_eq!(unread_count(""), 0);
    }
}
