// En release no abrimos una consola junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod badge;
mod links;
mod settings;
#[cfg(windows)]
mod toast;
mod updates;

use std::path::PathBuf;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::{DownloadEvent, NewWindowResponse, PermissionKind, PermissionResponse};
use tauri::{
    AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
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
/// Base del icono de la bandeja con el número de chats sin leer encima.
#[cfg(windows)]
const TRAY_BASE_ICON: &[u8] = include_bytes!("../icons/64x64.png");
/// En Linux la bandeja muestra un punto rojo y el número va en el lanzador (`set_badge_count`).
#[cfg(not(windows))]
const TRAY_UNREAD_ICON: &[u8] = include_bytes!("../icons/tray-unread.png");
/// Se inyecta en WhatsApp Web al cargar:
/// - Oculta el botón "Obtener WhatsApp para Windows" del pie de la lista de chats. Si WhatsApp
///   cambia su HTML, el selector deja de coincidir y simplemente vuelve a verse.
/// - WhatsApp cancela el clic en los enlaces para abrirlos con su propio código, que dentro de la
///   webview no hace nada. Nos adelantamos y los abrimos con `window.open`, que llega a
///   `on_new_window` y de ahí al navegador del sistema.
const INIT_SCRIPT: &str = r#"
document.addEventListener("DOMContentLoaded", () => {
  const style = document.createElement("style");
  style.textContent = `#side > [role="button"]:has([data-icon="wa-square-icon"]) { display: none !important; }`;
  document.head.append(style);
});
window.addEventListener("click", (event) => {
  const link = event.target.closest?.("a[href]");
  if (!link || event.button !== 0) return;
  const url = new URL(link.href, location.href);
  if (!/^https?:$/.test(url.protocol) || url.host === location.host) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  window.open(url.href, "_blank", "noopener");
}, true);
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
        .invoke_handler(tauri::generate_handler![
            settings::get_settings,
            settings::save_settings,
            settings::check_updates
        ])
        .on_window_event(|window, event| {
            let Some(window) = window
                .get_webview_window(window.label())
                .filter(|window| window.label() == MAIN_WINDOW)
            else {
                return;
            };
            match event {
                // Según el ajuste, cerrar la manda a la bandeja o sale de la app.
                WindowEvent::CloseRequested { api, .. } => {
                    if settings::get(window.app_handle()).close_to_tray {
                        api.prevent_close();
                        hide_window(&window);
                    } else {
                        window.app_handle().exit(0);
                    }
                }
                WindowEvent::Resized(_) => {
                    set_background(&window, window.is_minimized().unwrap_or(false));
                }
                _ => {}
            }
        })
        .setup(|app| {
            settings::init(app.handle());
            #[cfg(windows)]
            toast::register(app.handle())?;
            let start_hidden = std::env::args().any(|arg| arg == MINIMIZED_ARG)
                && settings::get(app.handle()).start_hidden;
            let window = build_main_window(app.handle(), !start_hidden)?;
            // En Linux WebKitGTK ya envía las notificaciones web al sistema.
            #[cfg(windows)]
            toast::attach(&window);
            if start_hidden {
                set_background(&window, true);
            }
            build_tray(app.handle())?;

            set_global_shortcut(app.handle(), settings::get(app.handle()).global_shortcut);

            updates::spawn_periodic_check(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Conversa");
}

/// Un directorio de datos por cuenta: deja preparada la multicuenta.
fn profile_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("profiles").join("default"))
}

fn set_global_shortcut(app: &AppHandle, enabled: bool) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister(TOGGLE_SHORTCUT);
    if !enabled {
        return;
    }
    if let Err(err) = shortcuts.on_shortcut(TOGGLE_SHORTCUT, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle_window(app);
        }
    }) {
        // Otra app puede tener ya el atajo: no es motivo para no arrancar.
        eprintln!("no se pudo registrar el atajo {TOGGLE_SHORTCUT}: {err}");
    }
}

fn build_main_window(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
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
    .data_directory(profile_dir(app)?)
    .initialization_script(INIT_SCRIPT)
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
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Mostrar / ocultar", true, None::<&str>)?,
            &MenuItem::with_id(app, "settings", "Ajustes", true, None::<&str>)?,
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
            "settings" => {
                if let Err(err) = settings::open_window(app) {
                    eprintln!("no se pudo abrir los ajustes: {err}");
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

/// `count` es el número de chats sin leer que WhatsApp Web pone en su título (sin los silenciados).
fn set_unread(window: &WebviewWindow, count: u32) {
    let label = match count {
        0 => "Conversa".to_string(),
        n => format!("({n}) Conversa"),
    };
    let _ = window.set_title(&label);

    if let Some(tray) = window.app_handle().tray_by_id(TRAY) {
        let _ = tray.set_icon(tray_icon(window.app_handle(), count));
        let _ = tray.set_tooltip(Some(&label));
    }

    #[cfg(windows)]
    let _ = window
        .set_overlay_icon((count > 0).then(|| Image::new_owned(badge::render(count, 32), 32, 32)));
    #[cfg(not(windows))]
    let _ = window.set_badge_count((count > 0).then_some(i64::from(count)));
}

fn tray_icon(app: &AppHandle, count: u32) -> Option<Image<'static>> {
    if count == 0 {
        return app.default_window_icon().cloned().map(Image::to_owned);
    }
    #[cfg(windows)]
    {
        const SIZE: u32 = 64;
        const BADGE: u32 = 40;
        let mut rgba = Image::from_bytes(TRAY_BASE_ICON).ok()?.rgba().to_vec();
        badge::overlay(&mut rgba, SIZE, &badge::render(count, BADGE), BADGE);
        Some(Image::new_owned(rgba, SIZE, SIZE))
    }
    #[cfg(not(windows))]
    Image::from_bytes(TRAY_UNREAD_ICON).ok()
}

/// Desde otro hilo: lanzado desde el hilo de WebView2, ShellExecute devuelve OK pero en Windows
/// no llega a abrir el navegador si no estaba ya abierto.
fn open_external(app: &AppHandle, url: &Url) {
    let (app, url) = (app.clone(), url.to_string());
    std::thread::spawn(move || {
        if let Err(err) = app.opener().open_url(&url, None::<&str>) {
            eprintln!("no se pudo abrir {url}: {err}");
        }
    });
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
