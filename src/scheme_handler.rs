//! Embedded `harbor://` scheme resolver and IPC router.
//!
//! Serves the one-row browser header (`harbor://chrome`) and internal workspace pages
//! (`harbor://home`, `harbor://bookmarks`, `harbor://history`, `harbor://logins`,
//! `harbor://adblock`, `harbor://scripts`, `harbor://vault`, `harbor://server`,
//! `harbor://settings`) directly from compiled binary assets with zero external network calls.

pub const CHROME_HTML: &str = include_str!("../ui/chrome.html");
pub const INTERNAL_HTML: &str = include_str!("../ui/internal.html");
const WORKSPACE_CSS: &str = include_str!("../ui/workspace.css");
const EMPTY_HTML: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8">
<title>No tabs open</title><style>
body { margin:0; min-height:100vh; display:grid; place-items:center; background:#181b20;
color:#9ca5b1; font:14px system-ui,sans-serif; }
</style><body><main>Open a tab with + or enter an address.</main></body></html>"#;

pub fn resolve_monaco_asset(url: &str) -> Option<(&'static [u8], &'static str)> {
    macro_rules! asset {
        ($file:literal, $mime:literal) => {
            Some((include_bytes!(concat!("../ui/monaco/", $file)), $mime))
        };
    }
    let path = url
        .strip_prefix("harbor://scripts/monaco/")
        .or_else(|| url.strip_prefix("harbor://install-script/monaco/"))?;
    match path {
        "vs/loader.js" => asset!("vs/loader.js", "text/javascript"),
        "vs/editor/editor.main.js" => asset!("vs/editor/editor.main.js", "text/javascript"),
        "vs/editor/editor.main.css" => asset!("vs/editor/editor.main.css", "text/css"),
        "vs/basic-languages/javascript/javascript.js" => asset!(
            "vs/basic-languages/javascript/javascript.js",
            "text/javascript"
        ),
        "vs/language/typescript/tsMode.js" => {
            asset!("vs/language/typescript/tsMode.js", "text/javascript")
        }
        "vs/language/typescript/tsWorker.js" => {
            asset!("vs/language/typescript/tsWorker.js", "text/javascript")
        }
        "vs/base/worker/workerMain.js" => asset!("vs/base/worker/workerMain.js", "text/javascript"),
        "vs/base/browser/ui/codicons/codicon/codicon.ttf" => asset!(
            "vs/base/browser/ui/codicons/codicon/codicon.ttf",
            "font/ttf"
        ),
        _ => None,
    }
}

pub struct SchemeResponse {
    pub mime_type: &'static str,
    pub body: &'static str,
}

impl SchemeResponse {
    pub fn render_body(&self) -> Vec<u8> {
        if self.body == INTERNAL_HTML {
            self.body
                .replace("/* HARBOR_WORKSPACE_CSS */", WORKSPACE_CSS)
                .replace("/* HARBOR_DASHBOARD */", include_str!("../ui/dashboard.js"))
                .into_bytes()
        } else {
            self.body.as_bytes().to_vec()
        }
    }
}

pub fn resolve_harbor_url(url: &str) -> Option<SchemeResponse> {
    let path = url.strip_prefix("harbor://")?;
    let route = path.split(['/', '?', '#']).next().unwrap_or("home");
    match route {
        "chrome" => Some(SchemeResponse {
            mime_type: "text/html",
            body: CHROME_HTML,
        }),
        "empty" => Some(SchemeResponse {
            mime_type: "text/html",
            body: EMPTY_HTML,
        }),
        "home" | "bookmarks" | "history" | "logins" | "adblock" | "scripts" | "extensions"
        | "vault" | "server" | "settings" | "setup" | "install-script" => Some(SchemeResponse {
            mime_type: "text/html",
            body: INTERNAL_HTML,
        }),
        _ => Some(SchemeResponse {
            mime_type: "text/html",
            body: INTERNAL_HTML,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_harbor_url;

    #[test]
    fn chrome_and_workspace_are_served_as_html() {
        for (url, title) in [
            ("harbor://chrome/", "<title>Harbor Chrome</title>"),
            ("harbor://home/", "<title>Harbor"),
        ] {
            let response = resolve_harbor_url(url).unwrap();
            assert_eq!(response.mime_type, "text/html");
            assert!(response.body.contains(title));
        }
    }

    #[test]
    fn workspace_embeds_assets_and_review_uses_same_origin_monaco() {
        let response = resolve_harbor_url("harbor://install-script/").unwrap();
        let body = String::from_utf8(response.render_body()).unwrap();
        assert!(body.contains(".field-action"));
        assert!(body.contains("function renderHomeDashboard"));
        assert!(!body.contains("/* HARBOR_WORKSPACE_CSS */"));
        assert!(
            super::resolve_monaco_asset("harbor://install-script/monaco/vs/loader.js").is_some()
        );
        assert!(super::resolve_monaco_asset("harbor://settings/monaco/vs/loader.js").is_none());
    }
}
