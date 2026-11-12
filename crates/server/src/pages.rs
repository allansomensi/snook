//! Public web pages: the privacy policy (Portuguese at `/privacidade`,
//! English at `/privacy`). Stores such as the Microsoft Store ask for its
//! address; see `docs/MSSTORE.md`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{Html, IntoResponse, Response};

use crate::Server;

const PRIVACY_PT: &str = include_str!("../pages/privacy.pt-BR.html");
const PRIVACY_EN: &str = include_str!("../pages/privacy.en.html");
/// The date shown at the top of the policy: change it with the text.
const PRIVACY_UPDATED: &str = "2026-11-12";

/// Who the policy names (from `SNOOK_GAME_NAME`, `SNOOK_PRIVACY_CONTROLLER`
/// and `SNOOK_PRIVACY_CONTACT`).
#[derive(Debug, Clone)]
pub struct PrivacyInfo {
    /// The game's name in the store.
    pub game: String,
    /// The person or company responsible for the data.
    pub controller: String,
    /// An e-mail address or a web address for privacy requests.
    pub contact: String,
}

impl Default for PrivacyInfo {
    fn default() -> Self {
        PrivacyInfo {
            game: "Snook".into(),
            controller: "Allan Somensi".into(),
            contact: format!("{}/issues", env!("CARGO_PKG_REPOSITORY")),
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The contact as a link: `mailto:` for an e-mail address.
fn contact_link(contact: &str) -> String {
    let c = contact.trim();
    let href = if c.contains('@') && !c.contains("://") {
        format!("mailto:{c}")
    } else if c.starts_with("https://") || c.starts_with("http://") {
        c.to_string()
    } else {
        return escape(c);
    };
    format!("<a href=\"{}\">{}</a>", escape(&href), escape(c))
}

pub fn render(template: &str, info: &PrivacyInfo) -> String {
    template
        .replace("{{GAME}}", &escape(&info.game))
        .replace("{{CONTROLLER}}", &escape(&info.controller))
        .replace("{{CONTACT}}", &contact_link(&info.contact))
        .replace("{{UPDATED}}", PRIVACY_UPDATED)
}

fn page(html: String) -> Response {
    let mut res = Html(html).into_response();
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    res
}

pub async fn privacy_pt(State(server): State<Arc<Server>>) -> Response {
    page(render(PRIVACY_PT, &server.config.privacy))
}

pub async fn privacy_en(State(server): State<Arc<Server>>) -> Response {
    page(render(PRIVACY_EN, &server.config.privacy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_fill_every_placeholder_and_escape_values() {
        let info = PrivacyInfo {
            game: "Pool <Club>".into(),
            controller: "Ana & Bia".into(),
            contact: "privacy@example.com".into(),
        };
        for template in [PRIVACY_PT, PRIVACY_EN] {
            let html = render(template, &info);
            assert!(!html.contains("{{"), "a placeholder was left");
            assert!(html.contains("Pool &lt;Club&gt;") && html.contains("Ana &amp; Bia"));
            assert!(html.contains("href=\"mailto:privacy@example.com\""));
        }
        assert_eq!(
            contact_link("https://x.org/a"),
            "<a href=\"https://x.org/a\">https://x.org/a</a>"
        );
        assert_eq!(contact_link("javascript:alert(1)"), "javascript:alert(1)");
    }
}
