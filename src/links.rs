//! Decide qué hacer con las URLs que WhatsApp Web intenta abrir y traduce los enlaces de WhatsApp
//! (`wa.me`, `whatsapp://`...) a WhatsApp Web.

use tauri::Url;

const WHATSAPP_WEB_HOST: &str = "web.whatsapp.com";

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Se queda dentro de la app (la propia WhatsApp Web, blobs de multimedia...).
    Internal,
    /// Se abre con la aplicación predeterminada del sistema (navegador, correo...).
    External,
    /// Enlace de WhatsApp (`wa.me`, invitación a grupo...): se abre dentro de la app, en esta URL
    /// equivalente de WhatsApp Web.
    Chat(Url),
    /// `conversa://settings`: el botón de Ajustes que inyectamos en la barra lateral de WhatsApp.
    /// Es una navegación que interceptamos, así la página no necesita acceso a la API de la app.
    AppSettings,
    /// Esquemas desconocidos: no se abren en ningún sitio.
    Blocked,
}

pub fn classify(url: &Url) -> Target {
    if let Some(web) = to_whatsapp_web(url) {
        return Target::Chat(web);
    }
    match url.scheme() {
        "http" | "https" if url.host_str() == Some(WHATSAPP_WEB_HOST) => Target::Internal,
        "http" | "https" | "mailto" | "tel" => Target::External,
        "about" | "blob" | "data" => Target::Internal,
        "conversa" => Target::AppSettings,
        _ => Target::Blocked,
    }
}

/// La URL de WhatsApp Web que abre lo mismo que un enlace de WhatsApp:
/// - `whatsapp://send?phone=X&text=Y`, `wa.me/X?text=Y`, `api.whatsapp.com/send?phone=X` → el chat
///   con ese número (y el texto ya escrito, sin enviar).
/// - `whatsapp://chat?code=C`, `chat.whatsapp.com/C` → la invitación a ese grupo.
///
/// `None` si no es un enlace de WhatsApp o es uno que WhatsApp Web no sabe abrir (p. ej. los
/// enlaces cortos de empresas `wa.me/message/...`).
pub fn to_whatsapp_web(url: &Url) -> Option<Url> {
    let query = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let host = url.host_str().unwrap_or_default();
    let first_segment = url
        .path_segments()
        .and_then(|mut segments| segments.next())
        .unwrap_or_default();
    let web_path = match (url.scheme(), host) {
        ("whatsapp", "send") => "send",
        ("whatsapp", "chat") => "accept",
        ("http" | "https", "wa.me") if first_segment.is_empty() || is_phone(first_segment) => {
            "send"
        }
        ("http" | "https", "api.whatsapp.com" | "www.whatsapp.com" | "whatsapp.com")
            if first_segment == "send" =>
        {
            "send"
        }
        ("http" | "https", "chat.whatsapp.com") if !first_segment.is_empty() => "accept",
        _ => return None,
    };

    let mut web = Url::parse(&format!("https://{WHATSAPP_WEB_HOST}/{web_path}")).ok()?;
    {
        let mut pairs = web.query_pairs_mut();
        if web_path == "send" {
            let phone = if host == "wa.me" {
                Some(first_segment.to_string())
            } else {
                query("phone")
            };
            if let Some(phone) = phone.filter(|phone| !phone.is_empty()) {
                pairs.append_pair("phone", &phone);
            }
            if let Some(text) = query("text") {
                pairs.append_pair("text", &text);
            }
        } else {
            let code = if url.scheme() == "whatsapp" {
                query("code")?
            } else {
                first_segment.to_string()
            };
            pairs.append_pair("code", &code);
        }
    }
    Some(web)
}

fn is_phone(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Target {
        classify(&url.parse().unwrap())
    }

    #[test]
    fn whatsapp_web_stays_inside() {
        assert_eq!(target("https://web.whatsapp.com/"), Target::Internal);
        assert_eq!(
            target("https://web.whatsapp.com/send?phone=34600000000"),
            Target::Internal
        );
    }

    #[test]
    fn other_sites_open_in_the_browser() {
        assert_eq!(target("https://example.com/article"), Target::External);
        assert_eq!(target("https://faq.whatsapp.com/"), Target::External);
        assert_eq!(
            target("https://web.whatsapp.com.evil.example/"),
            Target::External
        );
        assert_eq!(target("mailto:alguien@example.com"), Target::External);
        assert_eq!(target("tel:+34600000000"), Target::External);
    }

    #[test]
    fn page_internal_schemes_stay_inside() {
        assert_eq!(target("about:blank"), Target::Internal);
        assert_eq!(
            target("blob:https://web.whatsapp.com/0f3c8a2e-0000-4000-8000-000000000000"),
            Target::Internal
        );
    }

    fn web(url: &str) -> Option<String> {
        to_whatsapp_web(&url.parse().unwrap()).map(String::from)
    }

    #[test]
    fn chat_links_open_inside_whatsapp_web() {
        let expected = Some(
            "https://web.whatsapp.com/send?phone=34600000000&text=Hola+qu%C3%A9+tal".to_string(),
        );
        assert_eq!(
            web("whatsapp://send?phone=34600000000&text=Hola%20qu%C3%A9%20tal"),
            expected
        );
        assert_eq!(
            web("https://wa.me/34600000000?text=Hola%20qu%C3%A9%20tal"),
            expected
        );
        assert_eq!(
            web("https://api.whatsapp.com/send?phone=34600000000&text=Hola+qu%C3%A9+tal"),
            expected
        );
        assert_eq!(
            web(
                "https://api.whatsapp.com/send/?phone=34600000000&text=Hola+qu%C3%A9+tal&type=phone_number"
            ),
            expected
        );
        assert_eq!(
            web("https://wa.me/34600000000"),
            Some("https://web.whatsapp.com/send?phone=34600000000".into())
        );
        // Compartir un texto sin número: WhatsApp pregunta a quién enviarlo.
        assert_eq!(
            web("https://wa.me/?text=mira"),
            Some("https://web.whatsapp.com/send?text=mira".into())
        );
    }

    #[test]
    fn group_invites_open_inside_whatsapp_web() {
        let expected = Some("https://web.whatsapp.com/accept?code=AbCdEf123".to_string());
        assert_eq!(web("https://chat.whatsapp.com/AbCdEf123"), expected);
        assert_eq!(web("whatsapp://chat?code=AbCdEf123"), expected);
    }

    #[test]
    fn other_whatsapp_pages_are_not_chats() {
        assert_eq!(web("https://wa.me/message/ABCDEF"), None);
        assert_eq!(web("https://www.whatsapp.com/download"), None);
        assert_eq!(web("https://faq.whatsapp.com/"), None);
        assert_eq!(web("https://chat.whatsapp.com/"), None);
        assert_eq!(web("whatsapp://chat"), None);
        assert_eq!(web("https://wa.me.evil.example/34600000000"), None);
        assert_eq!(target("https://wa.me/message/ABCDEF"), Target::External);
    }

    #[test]
    fn classify_routes_chat_links() {
        assert_eq!(
            target("whatsapp://send?phone=1"),
            Target::Chat("https://web.whatsapp.com/send?phone=1".parse().unwrap())
        );
    }

    #[test]
    fn conversa_scheme_opens_settings() {
        assert_eq!(target("conversa://settings"), Target::AppSettings);
    }

    #[test]
    fn unknown_schemes_are_blocked() {
        assert_eq!(
            target("file:///C:/Windows/System32/calc.exe"),
            Target::Blocked
        );
        assert_eq!(target("ms-settings:privacy"), Target::Blocked);
        assert_eq!(target("javascript:alert(1)"), Target::Blocked);
    }
}
