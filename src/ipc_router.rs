//! Central JS-to-Rust IPC router for Harbor.
//!
//! Handles commands from both `harbor://chrome` (navigation, tabs, spaces, popovers, shield)
//! and `harbor://` internal pages (bookmarks, logins, adblock lists, user scripts, sync),
//! as well as content-script hooks (`offerSaveLogin`, `GM_getValue`, `GM_setValue`).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use shared_protocol::types::{AdblockConfigPayload, BookmarkPayload, HistoryPayload, ItemType};
use uuid::Uuid;

use crate::cef_shell::HarborRuntime;
use crate::tab_manager::{EMPTY_WORKSPACE, NO_TAB, SpaceKind, TabEntry};

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum IpcCommand {
    Navigate { input: String },
    NavBack,
    NavForward,
    NavHome,
    NavReload,
    TabNew,
    TabClose { id: u64 },
    TabSelect { id: u64 },
    SwitchSpace { space: String },
    ToggleSplit,
    ToggleReader,
    ToggleDevTools,
    PopoverOverlay { popover: Option<ChromePopover> },
    BookmarkToggle,
    AdblockToggleSite,
    WinMinimize,
    WinMaximize,
    WinClose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PopoverKind {
    Command,
    Shelf,
    Bookmarks,
    Menu,
    Extensions,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionAction {
    pub id: String,
    pub name: String,
    #[serde(rename = "popupUrl")]
    pub popup_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromePopover {
    pub kind: PopoverKind,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub query: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChromeStateSnapshot {
    pub space: String,
    #[serde(rename = "tabCount")]
    pub tab_count: usize,
    #[serde(rename = "activeId")]
    pub active_id: u64,
    pub tabs: Vec<TabEntry>,
    pub bookmarks: Vec<BookmarkPayload>,
    pub title: String,
    pub url: String,
    pub blocked: u64,
    #[serde(rename = "shieldOn")]
    pub shield_on: bool,
    pub progress: u8,
    pub split: bool,
    #[serde(rename = "showHomeButton")]
    pub show_home_button: bool,
    #[serde(rename = "canGoBack")]
    pub can_go_back: bool,
    #[serde(rename = "canGoForward")]
    pub can_go_forward: bool,
    pub extensions: Vec<ExtensionAction>,
}

/// Side-effect instructions returned to the CEF host layer after executing an IPC command.
#[derive(Debug, Clone)]
pub enum HostAction {
    None,
    LoadUrl { id: u64, url: String },
    SelectTab { id: u64, url: String },
    CloseTabView(u64),
    FocusTab(u64),
    RefreshSplit,
    GoBack,
    GoForward,
    Reload,
    ToggleReaderMode,
    ToggleDevTools,
    SetPopover(Option<ChromePopover>),
    WindowMinimize,
    WindowMaximize,
    WindowClose,
}

#[derive(Debug, Clone)]
pub struct IpcResult {
    pub host_action: HostAction,
    pub chrome_snapshot: ChromeStateSnapshot,
}

pub fn normalize_address_input(raw: &str, settings: &crate::workspace::HomeSettings) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "harbor://home".to_string();
    }
    if let Ok(url) = reqwest::Url::parse(trimmed) {
        if matches!(url.scheme(), "http" | "https")
            || (url.scheme() == "harbor" && url.host_str() != Some("chrome") && url.has_host())
        {
            return url.into();
        }
    }
    if !trimmed.contains("://") && !trimmed.contains(char::is_whitespace) {
        if let Ok(url) = reqwest::Url::parse(&format!("https://{trimmed}")) {
            if url.host_str().is_some_and(|host| host.contains('.') || host == "localhost")
                && url.username().is_empty()
                && url.password().is_none()
            {
                return url.into();
            }
        }
    }
    crate::workspace::search_address(settings, trimmed)
}

pub fn saved_bookmarks(runtime: &HarborRuntime) -> Vec<BookmarkPayload> {
    runtime.store.lock().expect("store lock")
        .list_by_type(ItemType::Bookmark)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|record| serde_json::from_str(&record.payload_json).ok())
        .collect()
}

pub fn build_chrome_snapshot(runtime: &HarborRuntime) -> ChromeStateSnapshot {
    let bookmarks = saved_bookmarks(runtime);
    let home = crate::workspace::home_settings(&runtime.store.lock().expect("store lock"));
    let tabs = runtime.tabs.lock().expect("tabs lock");
    let active = tabs.tab(tabs.focused_tab_id());
    let space = tabs.active_space;
    let shield_on = active.is_none_or(|tab| {
        !runtime.adblock.lock().expect("adblock lock").is_allowlisted(&tab.url)
    });
    let tab_count = tabs.space_tabs(space).len();
    let extensions = runtime
        .extensions
        .lock()
        .expect("extensions lock")
        .iter()
        .filter(|ext| ext.enabled && ext.workspace == space)
        .filter_map(|ext| {
            ext.popup_path.as_ref().map(|popup| ExtensionAction {
                id: ext.id.clone(),
                name: ext.name.clone(),
                popup_url: format!("harbor://extension-asset/{}/{}/{}", ext.id, ext.version, popup),
            })
        })
        .collect();
    ChromeStateSnapshot {
        space: space.as_str().to_string(),
        tab_count,
        active_id: tabs.focused_tab_id(),
        tabs: tabs.space_tabs(space).into_iter().cloned().collect(),
        bookmarks,
        title: active.map(|t| t.title.clone()).unwrap_or_else(|| "No tabs open".into()),
        url: active
            .map(|t| t.url.clone())
            .unwrap_or_default(),
        blocked: active.map(|t| t.blocked_on_page).unwrap_or(0),
        shield_on,
        progress: active.map(|t| t.loading_progress).unwrap_or(100),
        split: tabs.split_tab_id.is_some(),
        show_home_button: home.show_home_button,
        can_go_back: active.map(|t| t.can_go_back).unwrap_or(false),
        can_go_forward: active.map(|t| t.can_go_forward).unwrap_or(false),
        extensions,
    }
}

/// Serializes the current Harbor chrome state into a JS call for `harbor://chrome`.
pub fn chrome_update_js(runtime: &HarborRuntime) -> String {
    let snap = build_chrome_snapshot(runtime);
    let json = serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string());
    format!("window.__harborUpdateChrome && window.__harborUpdateChrome({json});")
}

/// Logs an external page visit into the encrypted local SQLite store for sync.
pub fn record_history_visit(runtime: &HarborRuntime, url: &str, title: &str) {
    if url.starts_with("harbor://") || url.is_empty() {
        return;
    }
    let now = now_unix();
    let payload = HistoryPayload {
        title: title.to_string(),
        url: url.to_string(),
        visited_at: now,
    };
    if let Ok(json) = serde_json::to_string(&payload) {
        let store = runtime.store.lock().expect("store lock");
        if store.upsert_lww(Uuid::new_v4(), ItemType::History, &json, false, true, now).is_ok() {
            runtime.sync.changed();
        }
    }
}

pub fn dispatch_ipc(runtime: &HarborRuntime, raw_json: &str) -> IpcResult {
    let parsed: Result<IpcCommand, _> = serde_json::from_str(raw_json);
    let Ok(cmd) = parsed else {
        tracing::warn!("invalid harbor IPC payload");
        return IpcResult {
            host_action: HostAction::None,
            chrome_snapshot: build_chrome_snapshot(runtime),
        };
    };

    let mut host_action = HostAction::None;

    match cmd {
        nav @ (IpcCommand::Navigate { .. } | IpcCommand::NavHome) => {
            let settings = crate::workspace::home_settings(&runtime.store.lock().expect("store lock"));
            let url = match nav {
                IpcCommand::Navigate { input } => normalize_address_input(&input, &settings),
                _ => settings.home_url,
            };
            let mut tabs = runtime.tabs.lock().expect("tabs lock");
            let id = if tabs.focused_tab_id() == NO_TAB {
                tabs.open_tab(&url, "New tab").id
            } else {
                tabs.focused_tab_id()
            };
            tabs.update_tab_navigation(id, Some(url.clone()), None, Some(15));
            host_action = HostAction::LoadUrl { id, url };
        }
        IpcCommand::NavBack => host_action = HostAction::GoBack,
        IpcCommand::NavForward => host_action = HostAction::GoForward,
        IpcCommand::NavReload => host_action = HostAction::Reload,
        IpcCommand::TabNew => {
            let entry = runtime
                .tabs
                .lock()
                .expect("tabs lock")
                .open_tab("harbor://home", "Home");
            host_action = HostAction::LoadUrl { id: entry.id, url: entry.url };
        }
        IpcCommand::TabClose { id } => {
            let mut tabs = runtime.tabs.lock().expect("tabs lock");
            let was_active = tabs.active_tab_id == id;
            let was_split = tabs.split_tab_id == Some(id);
            if tabs.close_tab(id) {
                if was_active {
                    host_action = match tabs.active_tab() {
                        Some(tab) => HostAction::SelectTab { id: tab.id, url: tab.url.clone() },
                        None => HostAction::LoadUrl { id: NO_TAB, url: EMPTY_WORKSPACE.into() },
                    };
                } else if was_split {
                    host_action = HostAction::RefreshSplit;
                } else {
                    host_action = HostAction::CloseTabView(id);
                }
            }
        }
        IpcCommand::TabSelect { id } => {
            let mut tabs = runtime.tabs.lock().expect("tabs lock");
            let visible = tabs.active_tab_id == id || tabs.split_tab_id == Some(id);
            if let Some(tab) = tabs.select_tab(id) {
                host_action = if visible {
                    HostAction::FocusTab(id)
                } else {
                    HostAction::SelectTab { id, url: tab.url.clone() }
                };
            }
        }
        IpcCommand::SwitchSpace { space } => {
            let kind = if space.eq_ignore_ascii_case("personal") {
                SpaceKind::Personal
            } else {
                SpaceKind::Work
            };
            let exts = runtime.extensions.lock().expect("extensions lock").clone();
            let dnr = crate::extensions::compile_extension_dnr_rules(&exts, &runtime.profile_dir, kind);
            runtime.adblock.lock().expect("adblock lock").set_extension_rules(&dnr);
            let mut tabs = runtime.tabs.lock().expect("tabs lock");
            host_action = match tabs.switch_space(kind) {
                Some(tab) => HostAction::SelectTab { id: tab.id, url: tab.url.clone() },
                None => HostAction::LoadUrl { id: NO_TAB, url: EMPTY_WORKSPACE.into() },
            };
        }
        IpcCommand::ToggleSplit => {
            let mut tabs = runtime.tabs.lock().expect("tabs lock");
            let was_empty = tabs.active_tab_id == NO_TAB;
            tabs.toggle_split_view();
            host_action = match tabs.active_tab().filter(|_| was_empty) {
                Some(tab) => HostAction::SelectTab { id: tab.id, url: tab.url.clone() },
                None => HostAction::RefreshSplit,
            };
        }
        IpcCommand::ToggleReader => host_action = HostAction::ToggleReaderMode,
        IpcCommand::ToggleDevTools => host_action = HostAction::ToggleDevTools,
        IpcCommand::PopoverOverlay { popover } => host_action = HostAction::SetPopover(popover),
        IpcCommand::BookmarkToggle => {
            let tabs = runtime.tabs.lock().expect("tabs lock");
            let active = tabs.tab(tabs.focused_tab_id()).cloned();
            drop(tabs);
            if let Some(tab) = active {
                let now = now_unix();
                let payload = BookmarkPayload {
                    title: tab.title,
                    url: tab.url,
                    folder: None,
                };
                if let Ok(json) = serde_json::to_string(&payload) {
                    let store = runtime.store.lock().expect("store lock");
                    if store.upsert_lww(
                        Uuid::new_v4(),
                        ItemType::Bookmark,
                        &json,
                        false,
                        true,
                        now,
                    ).is_ok() {
                        runtime.sync.changed();
                    }
                }
            }
        }
        IpcCommand::AdblockToggleSite => {
            let tabs = runtime.tabs.lock().expect("tabs lock");
            let url = tabs.tab(tabs.focused_tab_id())
                .map(|t| t.url.clone())
                .unwrap_or_default();
            drop(tabs);
            if let Ok(url) = reqwest::Url::parse(&url) {
                if let Some(host) = url.host_str().filter(|_| matches!(url.scheme(), "http" | "https")) {
                    let store = runtime.store.lock().expect("store lock");
                    let (list_urls, custom_rules, mut allowlisted_domains) = crate::load_saved_adblock_config(&store);
                    let enable = !allowlisted_domains.iter().any(|domain| domain == host);
                    if enable {
                        allowlisted_domains.push(host.to_string());
                    } else {
                        allowlisted_domains.retain(|domain| domain != host);
                    }
                    let config = AdblockConfigPayload { list_urls, custom_rules, allowlisted_domains };
                    if crate::workspace::store_adblock(&store, &config).is_ok() {
                        runtime.adblock.lock().expect("adblock lock").set_allowlisted(host, enable);
                        runtime.sync.changed();
                        runtime.filters_changed.notify_one();
                        host_action = HostAction::Reload;
                    }
                }
            }
        }
        IpcCommand::WinMinimize => host_action = HostAction::WindowMinimize,
        IpcCommand::WinMaximize => host_action = HostAction::WindowMaximize,
        IpcCommand::WinClose => host_action = HostAction::WindowClose,
    }

    IpcResult {
        host_action,
        chrome_snapshot: build_chrome_snapshot(runtime),
    }
}

#[cfg(test)]
mod tests {
    use super::IpcCommand;

    #[test]
    fn toolbar_accepts_window_commands_without_a_payload() {
        let parse = |json| serde_json::from_str::<IpcCommand>(json).unwrap();
        assert!(matches!(parse(r#"{"type":"win_minimize"}"#), IpcCommand::WinMinimize));
        assert!(matches!(parse(r#"{"type":"win_maximize"}"#), IpcCommand::WinMaximize));
        assert!(matches!(parse(r#"{"type":"win_close"}"#), IpcCommand::WinClose));
    }

    #[test]
    fn address_commands_support_web_and_workspace_navigation() {
        let settings = crate::workspace::HomeSettings::default();
        let normalize_address_input = |raw| super::normalize_address_input(raw, &settings);
        assert_eq!(normalize_address_input(" example.com "), "https://example.com/");
        assert_eq!(normalize_address_input("http://localhost:8080"), "http://localhost:8080/");
        assert_eq!(normalize_address_input("harbor://history"), "harbor://history");
        assert_eq!(
            normalize_address_input("morning & evening #1"),
            "https://duckduckgo.com/?q=morning+%26+evening+%231"
        );
        assert!(normalize_address_input("harbor://chrome/").starts_with("https://duckduckgo.com/"));
    }
}
