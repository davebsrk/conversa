//! Notificaciones nativas de Windows (con respuesta rápida) en lugar de las de WebView2.
//!
//! WhatsApp Web crea una notificación web → WebView2 nos la entrega (`NotificationReceived`) →
//! la marcamos como gestionada y mostramos un toast propio. Al pulsar el toast se lo decimos a
//! WhatsApp (`ReportClicked`), que abre ese chat; si además se escribió una respuesta, la
//! enviamos desde el cuadro de texto del chat una vez confirmado que es el chat correcto.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, Webview, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_24, ICoreWebView2Notification};
use webview2_com::{NotificationReceivedEventHandler, take_pwstr};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{IReference, TypedEventHandler};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastNotification, ToastNotificationManager,
};
use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegSetKeyValueW};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::core::{BOOL, HSTRING, IInspectable, Interface, PWSTR};

/// Igual que el identificador de la app: es el que pone el instalador NSIS a sus accesos directos.
const AUMID: &str = "io.github.davebsrk.conversa";
const REPLY_INPUT: &str = "reply";
const MAX_PENDING: usize = 100;
/// Intentos de enviar una respuesta (cada 200 ms) antes de rendirse y mostrar la ventana.
const REPLY_ATTEMPTS: u32 = 25;
const APP_ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Notificaciones web a las que aún podemos avisar de un clic. Solo se toca en el hilo principal.
    static PENDING: RefCell<VecDeque<(u64, ICoreWebView2Notification)>> = Default::default();
}

/// Da identidad a la app para los toasts: nombre e icono, incluso en la versión portable.
pub fn register(app: &AppHandle) -> tauri::Result<()> {
    let icon = app_icon_path(app)?;
    let key = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    for (name, value) in [
        ("DisplayName", "Conversa".into()),
        ("IconUri", icon.display().to_string()),
    ] {
        let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe {
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &key,
                &HSTRING::from(name),
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            );
        }
    }
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID));
    }
    Ok(())
}

/// Sustituye las notificaciones de WebView2 por toasts nativos.
pub fn attach(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let _ = window.with_webview(move |webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let Ok(core) = core.cast::<ICoreWebView2_24>() else {
            return;
        };
        let handler = NotificationReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let notification = args.Notification()?;
            args.SetHandled(true)?;

            let text = |get: unsafe fn(
                &ICoreWebView2Notification,
                *mut PWSTR,
            ) -> windows::core::Result<()>| {
                let mut value = PWSTR::null();
                get(&notification, &mut value)
                    .map(|_| take_pwstr(value))
                    .unwrap_or_default()
            };
            let toast = Toast {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                title: text(ICoreWebView2Notification::Title),
                body: text(ICoreWebView2Notification::Body),
                tag: text(ICoreWebView2Notification::Tag),
                icon: text(ICoreWebView2Notification::IconUri),
                silent: {
                    let mut silent = BOOL(0);
                    let _ = notification.IsSilent(&mut silent);
                    silent.as_bool()
                },
            };
            let _ = notification.ReportShown();
            PENDING.with_borrow_mut(|pending| {
                pending.push_back((toast.id, notification));
                if pending.len() > MAX_PENDING {
                    pending.pop_front();
                }
            });

            // Descargar la foto del contacto bloquea: fuera del hilo principal.
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(err) = toast.show(&app) {
                    eprintln!("no se pudo mostrar la notificación: {err}");
                }
            });
            Ok(())
        }));
        let mut token = 0;
        let _ = core.add_NotificationReceived(&handler, &mut token);
    });
}

struct Toast {
    id: u64,
    title: String,
    body: String,
    tag: String,
    icon: String,
    silent: bool,
}

impl Toast {
    fn show(self, app: &AppHandle) -> windows::core::Result<()> {
        let picture = download_picture(app, &self.tag, &self.icon)
            .or_else(|| app_icon_path(app).ok())
            .map(|path| {
                format!(
                    r#"<image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
                    xml_escape(&path.display().to_string())
                )
            })
            .unwrap_or_default();
        let audio = if self.silent {
            r#"<audio silent="true"/>"#
        } else {
            ""
        };
        let xml = format!(
            r#"<toast launch="open">
                <visual><binding template="ToastGeneric">
                    <text hint-maxLines="1">{title}</text>
                    <text>{body}</text>
                    {picture}
                </binding></visual>
                <actions>
                    <input id="{REPLY_INPUT}" type="text" placeHolderContent="Escribe una respuesta"/>
                    <action content="Enviar" arguments="reply" hint-inputId="{REPLY_INPUT}"/>
                </actions>
                {audio}
            </toast>"#,
            title = xml_escape(&self.title),
            body = if crate::settings::get(app).message_preview {
                xml_escape(&self.body)
            } else {
                "Nuevo mensaje".into()
            },
        );
        // Un toast por chat: el siguiente mensaje sustituye al anterior.
        let tag = (!self.tag.is_empty()).then(|| short_hash(&self.tag));
        let (app, id, title) = (app.clone(), self.id, self.title);
        show_xml(&xml, tag, move |reply| {
            let (app, title) = (app.clone(), title.clone());
            let _ = app
                .clone()
                .run_on_main_thread(move || activated(&app, id, title, reply));
        })
    }
}

/// Toast sencillo de la propia app (no de WhatsApp). `on_click` se llama al pulsar el toast o su
/// botón, desde un hilo cualquiera.
pub fn show_simple(
    title: &str,
    body: &str,
    button: Option<&str>,
    on_click: impl Fn() + Send + 'static,
) -> windows::core::Result<()> {
    let actions = button
        .map(|label| {
            format!(
                r#"<actions><action content="{}" arguments="button"/></actions>"#,
                xml_escape(label)
            )
        })
        .unwrap_or_default();
    let xml = format!(
        r#"<toast launch="open">
            <visual><binding template="ToastGeneric">
                <text>{}</text>
                <text>{}</text>
            </binding></visual>
            {actions}
        </toast>"#,
        xml_escape(title),
        xml_escape(body),
    );
    show_xml(&xml, Some("app".into()), move |_| on_click())
}

/// Muestra un toast; `on_activated` recibe el texto de la respuesta rápida si se escribió.
fn show_xml(
    xml: &str,
    tag: Option<String>,
    on_activated: impl Fn(Option<String>) + Send + 'static,
) -> windows::core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    if let Some(tag) = tag {
        toast.SetTag(&HSTRING::from(tag))?;
    }
    toast.Activated(&TypedEventHandler::new(
        move |_, args: windows::core::Ref<IInspectable>| {
            let reply = args
                .as_ref()
                .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|args| args.UserInput().ok())
                .and_then(|input| input.Lookup(&HSTRING::from(REPLY_INPUT)).ok())
                .and_then(|value| value.cast::<IReference<HSTRING>>().ok())
                .and_then(|value| value.Value().ok())
                .map(|value| value.to_string())
                .filter(|value| !value.trim().is_empty());
            on_activated(reply);
            Ok(())
        },
    ))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
}

/// En el hilo principal: avisa a WhatsApp del clic (abre el chat) y envía la respuesta si la hay.
fn activated(app: &AppHandle, id: u64, title: String, reply: Option<String>) {
    let notification = PENDING.with_borrow(|pending| {
        pending
            .iter()
            .find(|(pending_id, _)| *pending_id == id)
            .map(|(_, n)| n.clone())
    });
    if let Some(notification) = notification {
        unsafe {
            let _ = notification.ReportClicked();
        }
    }
    match (reply, app.get_webview(crate::MAIN_WINDOW)) {
        (Some(text), Some(webview)) => send_reply(webview, title, text, REPLY_ATTEMPTS),
        _ => crate::show_window(app),
    }
}

/// Escribe y envía la respuesta en el chat abierto, pero solo si su cabecera coincide con el
/// remitente de la notificación. Si no llega a abrirse, muestra la ventana para responder a mano.
fn send_reply(webview: Webview, title: String, text: String, attempts: u32) {
    let script = format!(
        r##"(() => {{
            const norm = s => s.replace(/[^\p{{L}}\p{{N}}]/gu, "").toLowerCase();
            const title = norm({title}), text = {text};
            const header = document.querySelector("#main header");
            const box = document.querySelector('#main footer [contenteditable="true"]');
            if (!title || !header || !box || !norm(header.textContent).includes(title)) return "wait";
            if (!box.textContent.includes(text)) {{
                box.focus();
                document.execCommand("insertText", false, text);
                return "typed";
            }}
            box.dispatchEvent(new KeyboardEvent("keydown", {{ key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true }}));
            return "sent";
        }})()"##,
        title = serde_json::to_string(&title).unwrap_or_default(),
        text = serde_json::to_string(&text).unwrap_or_default(),
    );
    let retry = webview.clone();
    let _ = webview.eval_with_callback(script, move |result| {
        if result.contains("sent") {
            return;
        }
        if attempts == 0 {
            let app = retry.app_handle().clone();
            let _ = retry.run_on_main_thread(move || crate::show_window(&app));
            return;
        }
        let (webview, title, text) = (retry.clone(), title.clone(), text.clone());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            send_reply(webview, title, text, attempts - 1);
        });
    });
}

/// La foto del contacto, en caché por chat. `None` si no es una URL http(s) o falla la descarga.
fn download_picture(app: &AppHandle, tag: &str, url: &str) -> Option<PathBuf> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return None;
    }
    let dir = app.path().app_cache_dir().ok()?.join("avatars");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!(
        "{}.jpg",
        short_hash(if tag.is_empty() { url } else { tag })
    ));
    unsafe {
        URLDownloadToFileW(
            None,
            &HSTRING::from(url),
            &HSTRING::from(path.as_os_str()),
            0,
            None,
        )
    }
    .ok()?;
    Some(path)
}

fn app_icon_path(app: &AppHandle) -> tauri::Result<PathBuf> {
    let path = app.path().app_data_dir()?.join("icon.png");
    // Se reescribe si cambia (p. ej. tras una actualización con icono nuevo).
    if std::fs::read(&path).ok().as_deref() != Some(APP_ICON) {
        std::fs::create_dir_all(path.parent().expect("tiene carpeta"))?;
        std::fs::write(&path, APP_ICON)?;
    }
    Ok(path)
}

fn short_hash(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::xml_escape;

    #[test]
    fn escapes_xml() {
        assert_eq!(
            xml_escape(r#"<b>"Tom" & 'Jerry'</b>"#),
            "&lt;b&gt;&quot;Tom&quot; &amp; &apos;Jerry&apos;&lt;/b&gt;"
        );
    }
}
