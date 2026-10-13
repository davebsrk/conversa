// En release no abrimos una consola junto a la ventana en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
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
use tauri::webview::{
    DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse,
};
use tauri::window::WindowBuilder;
use tauri::{
    AppHandle, Manager, Url, Webview, WebviewBuilder, WebviewUrl, Window, WindowEvent, Wry,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::StateFlags;

use links::Target;

const WHATSAPP_URL: &str = "https://web.whatsapp.com/";
const MAIN_WINDOW: &str = "main";
const TRAY: &str = "main";
/// Argumento con el que se lanza al iniciar sesión en el sistema: arranca oculta en la bandeja.
const MINIMIZED_ARG: &str = "--minimized";
/// Base del icono de la bandeja con el número de chats sin leer encima.
#[cfg(windows)]
const TRAY_BASE_ICON: &[u8] = include_bytes!("../icons/64x64.png");
/// En Linux la bandeja muestra un punto rojo y el número va en el lanzador (`set_badge_count`).
#[cfg(not(windows))]
const TRAY_UNREAD_ICON: &[u8] = include_bytes!("../icons/tray-unread.png");
/// Se inyecta en WhatsApp Web al cargar:
/// - Aplica el aspecto de los ajustes (privacidad, ancho de la lista, elementos ocultos) guardado
///   en `localStorage`.
/// - Oculta el botón "Obtener WhatsApp para Windows" del pie de la lista de chats. Si WhatsApp
///   cambia su HTML, el selector deja de coincidir y simplemente vuelve a verse.
/// - WhatsApp cancela el clic en los enlaces para abrirlos con su propio código, que dentro de la
///   webview no hace nada. Nos adelantamos y los abrimos con `window.open`, que llega a
///   `on_new_window` y de ahí al navegador del sistema.
/// - Añade un botón de Ajustes de Conversa a la barra lateral de WhatsApp, copiando el aspecto
///   del botón "Contenido multimedia". Al pulsarlo navega a `conversa://settings`, que
///   interceptamos en `on_navigation`. Si WhatsApp redibuja la barra, se vuelve a añadir.
const INIT_SCRIPT: &str = r##"
const ACCOUNT_COLORS = ["#059669", "#2563eb", "#d97706", "#db2777", "#7c3aed", "#0891b2"];
const GEAR_SVG = '<svg viewBox="0 0 24 24" height="24" width="24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>';
// Aspecto según los ajustes (privacidad, ancho de la lista, elementos ocultos). Rust lo envía con
// conversaApplyAppearance y se guarda en localStorage para aplicarlo aquí, antes de que se pinte
// nada, en cada carga de la página.
let conversaHidden = [];
let conversaAccounts = [];
window.conversaApplyAppearance = (appearance) => {
  try { localStorage.setItem("conversa-appearance", JSON.stringify(appearance)); } catch {}
  let style = document.getElementById("conversa-css");
  if (!style) {
    style = document.createElement("style");
    style.id = "conversa-css";
    (document.head ?? document.documentElement).append(style);
  }
  style.textContent = appearance.css;
  conversaHidden = appearance.hide;
  conversaAccounts = appearance.accounts ?? [];
  hideItems();
  renderAccounts();
};
(function applySavedAppearance() {
  if (!document.documentElement) return requestAnimationFrame(applySavedAppearance);
  try {
    localStorage.removeItem("conversa-css"); // formato de la 0.1.6
    const saved = JSON.parse(localStorage.getItem("conversa-appearance"));
    if (saved) window.conversaApplyAppearance(saved);
  } catch {}
})();
// Oculta botones de la barra lateral (por el título de su icono, que no depende del idioma) y la
// barra de filtros de la lista de chats. Se repite cada vez que WhatsApp redibuja.
// Con varias cuentas: un botón redondo por cuenta encima del engranaje. El de esta cuenta va
// marcado; los demás llevan un punto si tienen chats sin leer. Navegan a conversa://account/<id>.
function renderAccounts() {
  const gear = document.getElementById("conversa-settings");
  // Sin sesión (pantalla del QR) no hay barra lateral: los botones flotan abajo a la izquierda,
  // con un engranaje para llegar a Ajustes.
  if (!document.body) return; // aún cargando: el observador lo pintará después
  const floating = !document.querySelector("#side");
  let box = document.getElementById("conversa-accounts");
  const signature = JSON.stringify(conversaAccounts) + floating;
  if (conversaAccounts.length < 2 || (!gear && !floating)) return box?.remove();
  if (box?.dataset.signature === signature && (floating || box.nextElementSibling === gear)) return;
  box?.remove();
  box = document.createElement("div");
  box.id = "conversa-accounts";
  box.dataset.signature = signature;
  box.style.cssText = floating
    ? "position:fixed;bottom:16px;left:16px;z-index:9999;display:flex;flex-direction:column;gap:10px;align-items:center;padding:8px;border-radius:24px;background:rgba(0,0,0,.35);"
    : "display:flex;flex-direction:column;align-items:center;gap:10px;padding:8px 0 4px;";
  conversaAccounts.forEach((account, i) => {
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.account = account.id;
    const label = account.unread ? `${account.name} (${account.unread} sin leer)` : account.name;
    button.title = account.current ? `${label} — cuenta actual` : `Cambiar a ${label}`;
    button.setAttribute("aria-label", button.title);
    button.textContent = ([...account.name.trim()][0] ?? "?").toUpperCase();
    button.style.cssText = `position:relative;width:32px;height:32px;border-radius:50%;border:0;padding:0;cursor:pointer;font:600 14px/32px system-ui,sans-serif;color:#fff;background:${ACCOUNT_COLORS[i % ACCOUNT_COLORS.length]};` +
      (account.current ? "box-shadow:0 0 0 2px #fff,0 0 0 4px #10b981;" : "opacity:.8;");
    if (account.unread && !account.current) {
      const dot = document.createElement("span");
      dot.style.cssText = "position:absolute;top:-2px;right:-2px;width:11px;height:11px;border-radius:50%;background:#e5383b;border:2px solid #fff;";
      button.append(dot);
    }
    button.addEventListener("click", () => { if (!account.current) location.href = "conversa://account/" + account.id; });
    box.append(button);
  });
  if (floating) {
    const settings = document.createElement("button");
    settings.type = "button";
    settings.title = "Ajustes de Conversa";
    settings.setAttribute("aria-label", settings.title);
    settings.innerHTML = GEAR_SVG;
    settings.style.cssText = "width:32px;height:32px;border:0;border-radius:50%;padding:4px;cursor:pointer;color:#fff;background:transparent;";
    settings.addEventListener("click", () => { location.href = "conversa://settings"; });
    box.append(settings);
    document.body.append(box);
  } else {
    gear.before(box);
  }
}
// El ancho de la lista: WhatsApp reserva ese ancho dos veces, en la columna de la lista (padre de
// #side) y en un hueco vacío al principio de la capa superpuesta que contiene el chat abierto. Se
// marcan los dos con data-conversa-list para que el CSS de los ajustes les dé el mismo ancho.
function markListColumns() {
  const column = document.querySelector("#side")?.parentElement;
  if (!column) return;
  column.dataset.conversaList = "";
  for (const layer of column.parentElement.children) {
    if (layer === column || layer.children.length !== 2 || getComputedStyle(layer).position !== "absolute") continue;
    const spacer = layer.children[0];
    if (spacer.dataset.conversaList !== undefined) continue;
    const style = getComputedStyle(spacer);
    if (style.flexGrow === "0" && style.flexBasis.endsWith("%")) spacer.dataset.conversaList = "";
  }
}
function outermost(el, isSingle) {
  while (el?.parentElement && isSingle(el.parentElement)) el = el.parentElement;
  return el;
}
function hideItems() {
  const rail = document.querySelector('header[data-testid="chatlist-header"]');
  for (const title of rail?.querySelectorAll("button[data-navbar-item] svg title") ?? []) {
    const item = outermost(title.closest("button"), (p) => p.querySelectorAll("button[data-navbar-item]").length === 1);
    item.style.display = conversaHidden.includes(title.textContent) ? "none" : "";
  }
  const filters = document.querySelector('#side [role="tablist"][aria-label="chat-list-filters"]');
  if (filters) outermost(filters, (p) => p.children.length === 1).style.display = conversaHidden.includes("filters") ? "none" : "";
}
document.addEventListener("DOMContentLoaded", () => {
  const style = document.createElement("style");
  style.textContent = `#side > [role="button"]:has([data-icon="wa-square-icon"]) { display: none !important; }`;
  document.head.append(style);
});
function addSettingsButton() {
  const media = document.querySelector('header[data-testid="chatlist-header"] button[data-navbar-item-index="4"]');
  if (!media || document.getElementById("conversa-settings")) return;
  let item = media;
  while (item.parentElement && item.parentElement.children.length === 1) item = item.parentElement;
  const clone = item.cloneNode(true);
  clone.id = "conversa-settings";
  const button = clone.querySelector("button");
  button.setAttribute("aria-label", "Ajustes de Conversa");
  button.title = "Ajustes de Conversa";
  button.removeAttribute("data-navbar-item-index");
  button.querySelector("svg").outerHTML = GEAR_SVG;
  button.addEventListener("click", () => { location.href = "conversa://settings"; });
  item.before(clone);
}
document.addEventListener("DOMContentLoaded", () => {
  let scheduled = false;
  new MutationObserver(() => {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(() => { scheduled = false; addSettingsButton(); hideItems(); renderAccounts(); markListColumns(); });
  }).observe(document.body, { childList: true, subtree: true });
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
"##;

/// WhatsApp Web rechaza WebKitGTK como "navegador no soportado", así que en Linux nos
/// presentamos como un Chrome de escritorio. En Windows WebView2 ya envía un UA de Edge válido.
#[cfg(target_os = "linux")]
const LINUX_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

fn main() {
    tauri::Builder::default()
        // Debe registrarse el primero: si ya hay una instancia, mostramos su ventana y salimos.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        // Enlaces whatsapp:// del sistema; con la feature "deep-link" de single-instance también
        // llegan los que se abren con Conversa ya en marcha.
        .plugin(tauri_plugin_deep_link::init())
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
            settings::check_updates,
            settings::install_update,
            settings::close_settings,
            settings::pause_shortcuts,
            settings::resume_shortcuts,
            settings::add_account,
            settings::switch_account,
            settings::remove_account
        ])
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW {
                return;
            }
            let app = window.app_handle();
            match event {
                // Según el ajuste, cerrar la manda a la bandeja o sale de la app.
                WindowEvent::CloseRequested { api, .. } => {
                    if settings::get(app).close_to_tray {
                        api.prevent_close();
                        hide_window(app);
                    } else {
                        app.exit(0);
                    }
                }
                WindowEvent::Resized(_) => {
                    refresh_memory(app);
                    settings::resize_panel(window);
                }
                _ => {}
            }
        })
        .setup(|app| {
            settings::init(app.handle());
            app.manage(accounts::Unread::default());
            app.manage(updates::PendingUpdate::default());
            accounts::clean_removed_profiles(app.handle());
            #[cfg(windows)]
            toast::register(app.handle())?;
            let start_hidden = std::env::args().any(|arg| arg == MINIMIZED_ARG)
                && settings::get(app.handle()).start_hidden;
            // Arrancada con un enlace de WhatsApp (whatsapp://...): la ventana se crea ya en ese chat.
            // Navegar a él después, durante la primera carga, deja WebView2 colgado en about:blank.
            let start_url = app
                .deep_link()
                .get_current()
                .ok()
                .flatten()
                .and_then(|urls| urls.iter().find_map(links::to_whatsapp_web))
                .unwrap_or_else(|| WHATSAPP_URL.parse().expect("URL válida"));
            let window = build_window(app.handle(), !start_hidden)?;
            let settings = settings::get(app.handle());
            for account in &settings.accounts {
                let active = account.id == settings.active_account;
                let url = if active {
                    start_url.clone()
                } else {
                    WHATSAPP_URL.parse().expect("URL válida")
                };
                accounts::create(app.handle(), &window, account, url, active)?;
            }
            refresh_memory(app.handle());
            build_tray(app.handle())?;

            // Otra app puede tener ya un atajo: no es motivo para no arrancar.
            if let Err(err) = set_shortcuts(app.handle(), &settings::get(app.handle())) {
                eprintln!("{err}");
            }

            updates::spawn_periodic_check(app.handle().clone());

            let deep_link = app.deep_link();
            // El instalador ya registra whatsapp://; esto cubre la versión portable. En desarrollo no,
            // para no robarle el registro a la Conversa instalada.
            if !cfg!(debug_assertions)
                && let Err(err) = deep_link.register_all()
            {
                eprintln!("no se pudo registrar whatsapp://: {err}");
            }
            let handle = app.handle().clone();
            deep_link.on_open_url(move |event| open_whatsapp_links(&handle, event.urls()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Conversa");
}

/// Lo que hace un atajo global.
type Action = fn(&AppHandle);

/// Registra los atajos globales de los ajustes. Si uno no se puede usar (formato inválido u otra app
/// ya lo tiene) devuelve un mensaje para mostrar en Ajustes.
fn set_shortcuts(app: &AppHandle, settings: &settings::Settings) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let actions: [(bool, &str, Action); 2] = [
        (
            settings.global_shortcut,
            &settings.toggle_shortcut,
            toggle_window,
        ),
        (true, &settings.privacy_shortcut, settings::toggle_privacy),
    ];
    for (enabled, accelerator, action) in actions {
        if !enabled || accelerator.is_empty() {
            continue;
        }
        shortcuts
            .on_shortcut(accelerator, move |app, _shortcut, event| {
                if event.state == ShortcutState::Pressed {
                    action(app);
                }
            })
            .map_err(|err| shortcut_error(accelerator, &err.to_string()))?;
    }
    Ok(())
}

/// Mensaje para el usuario, con la combinación legible ("Ctrl + Alt + J").
fn shortcut_error(accelerator: &str, err: &str) -> String {
    let pretty = accelerator
        .split('+')
        .map(|part| part.trim_start_matches("Key").trim_start_matches("Digit"))
        .collect::<Vec<_>>()
        .join(" + ");
    if err.contains("already registered") {
        format!("No se pudo usar el atajo {pretty}: ya lo usa otra aplicación.")
    } else {
        format!("No se pudo usar el atajo {pretty}: no es una combinación válida.")
    }
}

fn pause_shortcuts(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
}

/// La ventana principal; cada cuenta de WhatsApp es una webview dentro de ella (ver `accounts`).
fn build_window(app: &AppHandle, visible: bool) -> tauri::Result<Window> {
    WindowBuilder::new(app, MAIN_WINDOW)
        .title("Conversa")
        .inner_size(1100.0, 750.0)
        .min_inner_size(400.0, 500.0)
        .visible(visible)
        .build()
}

/// La webview de WhatsApp Web de una cuenta, con su propia carpeta de datos (sesión).
fn whatsapp_webview(app: &AppHandle, id: &str, url: Url) -> tauri::Result<WebviewBuilder<Wry>> {
    let on_nav = app.clone();
    let on_new = app.clone();

    let builder = WebviewBuilder::new(accounts::label(id), WebviewUrl::External(url))
        .data_directory(accounts::profile_dir(app, id)?)
        .initialization_script(INIT_SCRIPT)
        .on_navigation(move |url| match links::classify(url) {
            Target::Internal => true,
            Target::External => {
                open_external(&on_nav, url);
                false
            }
            Target::Chat(web) => {
                open_chat(&on_nav, web);
                false
            }
            Target::AppSettings => {
                open_settings(&on_nav);
                false
            }
            Target::SwitchAccount(id) => {
                switch_account(&on_nav, id);
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
            Target::Chat(web) => {
                open_chat(&on_new, web);
                NewWindowResponse::Deny
            }
            Target::AppSettings => {
                open_settings(&on_new);
                NewWindowResponse::Deny
            }
            Target::SwitchAccount(id) => {
                switch_account(&on_new, id);
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
        .on_document_title_changed(|webview, title| {
            if let Some(id) = accounts::id_of(webview.label()) {
                let app = webview.app_handle();
                accounts::set_unread(app, id, unread_count(&title));
                refresh_unread(app);
            }
        })
        // Resincroniza el aspecto con settings.json en cada carga (el de localStorage puede ser antiguo).
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                settings::apply_appearance_to(&webview, &settings::get(webview.app_handle()));
            }
        });

    #[cfg(target_os = "linux")]
    let builder = builder.user_agent(LINUX_USER_AGENT);

    Ok(builder)
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
            "settings" => open_settings(app),
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

fn open_whatsapp_links(app: &AppHandle, urls: Vec<Url>) {
    if let Some(web) = urls.iter().find_map(links::to_whatsapp_web) {
        open_chat(app, web);
    }
}

/// Abre un enlace de WhatsApp en la ventana principal. Desde otro hilo por lo mismo que
/// `open_settings`: se puede llamar desde dentro de un manejador de navegación de WebView2.
fn open_chat(app: &AppHandle, web: Url) {
    let app = app.clone();
    std::thread::spawn(move || {
        show_window(&app);
        if let Some(webview) = accounts::active(&app) {
            let _ = webview.navigate(web);
        }
    });
}

/// Desde otro hilo, por lo mismo que `open_settings`.
fn switch_account(app: &AppHandle, id: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        accounts::switch(&app, &id);
        settings::apply_appearance(&app, &settings::get(&app));
    });
}

/// Desde otro hilo, para que Tauri lo encole: creada dentro de un manejador de WebView2 (p. ej. la
/// navegación a `conversa://settings`) la nueva webview se queda en blanco.
fn open_settings(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(err) = settings::open_panel(&app) {
            eprintln!("no se pudo abrir los ajustes: {err}");
        }
    });
}

// La ventana principal y sus webviews (una por cuenta, más el panel de Ajustes) se buscan por
// separado: con varias webviews `get_webview_window` no encuentra la ventana.
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    set_background(app, false);
}

fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
    set_background(app, true);
}

fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_window(MAIN_WINDOW) else {
        return;
    };
    let in_front = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if in_front {
        hide_window(app);
    } else {
        show_window(app);
    }
}

/// Modo híbrido: con la ventana oculta o minimizada, y para las cuentas que no se están viendo,
/// pedimos a WebView2 que libere toda la memoria posible, pero sin suspender la página, para que
/// sigan llegando mensajes y notificaciones al momento.
fn set_background(app: &AppHandle, background: bool) {
    let active = accounts::label(&settings::get(app).active_account);
    for webview in accounts::all(app) {
        set_memory_low(&webview, background || webview.label() != active);
    }
}

/// `set_background` según el estado actual de la ventana.
fn refresh_memory(app: &AppHandle) {
    let background = app.get_window(MAIN_WINDOW).is_none_or(|window| {
        !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false)
    });
    set_background(app, background);
}

fn set_memory_low(webview: &Webview, low: bool) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
        };
        use windows::core::Interface;

        let level = if low {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        let _ = webview.with_webview(move |webview| unsafe {
            if let Ok(core) = webview.controller().CoreWebView2()
                && let Ok(core) = core.cast::<ICoreWebView2_19>()
            {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        });
    }
    // ponytail: WebKitGTK no tiene un equivalente directo; en Linux no hacemos nada de momento.
    #[cfg(not(windows))]
    let _ = (webview, low);
}

/// WhatsApp Web pone los no leídos en el título: "(3) WhatsApp".
fn unread_count(title: &str) -> u32 {
    title
        .strip_prefix('(')
        .and_then(|rest| rest.split_once(')'))
        .and_then(|(count, _)| count.parse().ok())
        .unwrap_or(0)
}

/// Título, bandeja e icono de la barra de tareas con el total de chats sin leer de todas las cuentas
/// (lo que WhatsApp Web pone en su título: sin los silenciados). Las otras cuentas lo muestran en
/// sus botones de la barra lateral.
fn refresh_unread(app: &AppHandle) {
    let settings = settings::get(app);
    let count = settings
        .accounts
        .iter()
        .map(|account| accounts::unread(app, &account.id))
        .sum::<u32>();
    let Some(window) = app.get_window(MAIN_WINDOW) else {
        return;
    };
    let label = match count {
        0 => "Conversa".to_string(),
        n => format!("({n}) Conversa"),
    };
    let _ = window.set_title(&label);

    if let Some(tray) = app.tray_by_id(TRAY) {
        let _ = tray.set_icon(tray_icon(app, count));
        let _ = tray.set_tooltip(Some(&label));
    }
    if settings.accounts.len() > 1 {
        settings::apply_appearance(app, &settings);
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
    fn shortcut_errors_are_readable() {
        assert_eq!(
            super::shortcut_error("Ctrl+Alt+KeyJ", "HotKey already registered: HotKey { .. }"),
            "No se pudo usar el atajo Ctrl + Alt + J: ya lo usa otra aplicación."
        );
        assert_eq!(
            super::shortcut_error("Ctrl+Digit1", "Couldn't recognize \"Foo\" as a valid key"),
            "No se pudo usar el atajo Ctrl + 1: no es una combinación válida."
        );
    }

    #[test]
    fn reads_unread_count_from_title() {
        assert_eq!(unread_count("(3) WhatsApp"), 3);
        assert_eq!(unread_count("(120) WhatsApp Business"), 120);
        assert_eq!(unread_count("WhatsApp"), 0);
        assert_eq!(unread_count("(x) WhatsApp"), 0);
        assert_eq!(unread_count(""), 0);
    }
}
