//! Decide qué hacer con las URLs que WhatsApp Web intenta abrir.

use tauri::Url;

const WHATSAPP_WEB_HOST: &str = "web.whatsapp.com";

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// Se queda dentro de la app (la propia WhatsApp Web, blobs de multimedia...).
    Internal,
    /// Se abre con la aplicación predeterminada del sistema (navegador, correo...).
    External,
    /// Esquemas desconocidos: no se abren en ningún sitio.
    Blocked,
}

pub fn classify(url: &Url) -> Target {
    match url.scheme() {
        "http" | "https" if url.host_str() == Some(WHATSAPP_WEB_HOST) => Target::Internal,
        "http" | "https" | "mailto" | "tel" => Target::External,
        "about" | "blob" | "data" => Target::Internal,
        _ => Target::Blocked,
    }
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
