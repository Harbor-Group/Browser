use std::time::Duration;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shared_protocol::{
    crypto::hash_token,
    types::{AdblockConfigPayload, ItemType, LoginPayload, SettingPayload, UserscriptPayload},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    cef_shell::HarborRuntime,
    ipc_router::now_unix,
    userscripts::{MAX_SCRIPT_BYTES, UserScript, review},
};

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WorkspaceCommand {
    State,
    SyncStatus,
    SkipSetup,
    CheckServer {
        server_url: String,
        pin: String,
    },
    Pair {
        server_url: String,
        pin: String,
        device_name: String,
        code: String,
        passphrase: String,
    },
    Unlock {
        passphrase: String,
    },
    SyncNow,
    ReviewScript {
        url: String,
    },
    InstallScript {
        token: Uuid,
    },
    SaveScript {
        id: Option<Uuid>,
        source: String,
    },
    ScriptDetails {
        id: Uuid,
    },
    SetScriptEnabled {
        id: Uuid,
        enabled: bool,
    },
    DeleteRecord {
        id: Uuid,
    },
    SaveBookmark {
        id: Option<Uuid>,
        title: String,
        url: String,
        folder: Option<Uuid>,
    },
    MoveBookmark {
        id: Uuid,
        folder: Option<Uuid>,
    },
    SaveFolder {
        id: Option<Uuid>,
        name: String,
    },
    DeleteFolder {
        id: Uuid,
    },
    ClearHistory {
        from: Option<i64>,
        until: Option<i64>,
    },
    SaveLogin {
        id: Option<Uuid>,
        origin: String,
        username: String,
        password: String,
    },
    LoginPassword {
        id: Uuid,
    },
    SaveAdblock {
        lists: Vec<String>,
        rules: Vec<String>,
    },
    SaveHomeSettings {
        show_home_button: bool,
        home_url: String,
        search_engine: SearchEngine,
        custom_search_url: String,
        #[serde(default)]
        dns_preset: DnsPreset,
        #[serde(default)]
        custom_dns_url: String,
        #[serde(default = "default_true")]
        background_tabs_enabled: bool,
        #[serde(default = "default_bg_tab_limit")]
        background_tab_limit: u32,
        #[serde(default = "default_sleep_minutes")]
        sleep_tabs_after_minutes: u32,
        #[serde(default = "default_true")]
        sleep_preserve_audio: bool,
    },
    ReviewExtension {
        path: String,
        workspace: crate::tab_manager::SpaceKind,
    },
    InstallExtension {
        token: Uuid,
    },
    SetExtensionEnabled {
        id: String,
        enabled: bool,
    },
    RemoveExtension {
        id: String,
    },
}

fn default_true() -> bool {
    true
}

fn default_bg_tab_limit() -> u32 {
    8
}

fn default_sleep_minutes() -> u32 {
    30
}

#[derive(Serialize, Deserialize)]
pub struct HomeSettings {
    pub show_home_button: bool,
    pub home_url: String,
    #[serde(default)]
    pub search_engine: SearchEngine,
    #[serde(default)]
    pub custom_search_url: String,
    #[serde(default)]
    pub dns_preset: DnsPreset,
    #[serde(default)]
    pub custom_dns_url: String,
    #[serde(default = "default_true")]
    pub background_tabs_enabled: bool,
    #[serde(default = "default_bg_tab_limit")]
    pub background_tab_limit: u32,
    #[serde(default = "default_sleep_minutes")]
    pub sleep_tabs_after_minutes: u32,
    #[serde(default = "default_true")]
    pub sleep_preserve_audio: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DnsPreset {
    #[default]
    Automatic,
    Cloudflare,
    Quad9,
    Google,
    Adguard,
    Mullvad,
    Off,
    Custom,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Google,
    Bing,
    Brave,
    Custom,
}

impl Default for HomeSettings {
    fn default() -> Self {
        Self {
            show_home_button: false,
            home_url: "harbor://home".into(),
            search_engine: SearchEngine::default(),
            custom_search_url: String::new(),
            dns_preset: DnsPreset::default(),
            custom_dns_url: String::new(),
            background_tabs_enabled: default_true(),
            background_tab_limit: default_bg_tab_limit(),
            sleep_tabs_after_minutes: default_sleep_minutes(),
            sleep_preserve_audio: default_true(),
        }
    }
}

pub fn dns_over_https_config(settings: &HomeSettings) -> (&'static str, String) {
    match settings.dns_preset {
        DnsPreset::Off => ("off", String::new()),
        DnsPreset::Automatic => ("automatic", String::new()),
        DnsPreset::Cloudflare => ("secure", "https://cloudflare-dns.com/dns-query".into()),
        DnsPreset::Quad9 => ("secure", "https://dns.quad9.net/dns-query".into()),
        DnsPreset::Google => ("secure", "https://dns.google/dns-query".into()),
        DnsPreset::Adguard => ("secure", "https://dns.adguard-dns.com/dns-query".into()),
        DnsPreset::Mullvad => ("secure", "https://doh.mullvad.net/dns-query".into()),
        DnsPreset::Custom => ("secure", settings.custom_dns_url.clone()),
    }
}

fn valid_doh_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim()).ok()?;
    (url.scheme() == "https"
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.into())
}

const HOME_SETTING: &str = "harbor:home-button";

fn custom_search_address(template: &str, query: &str) -> Option<String> {
    if !template.contains("{query}") {
        return None;
    }
    let authority = template
        .split_once("://")?
        .1
        .split(['/', '?', '#'])
        .next()?;
    if authority.contains("{query}") {
        return None;
    }
    let mut encoding = Url::parse("https://search.invalid/").unwrap();
    encoding.query_pairs_mut().append_pair("q", query);
    let encoded = encoding.query()?.strip_prefix("q=")?.replace('+', "%20");
    let address = template.replace("{query}", &encoded);
    let url = Url::parse(&address).ok()?;
    (url.scheme() == "https"
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.into())
}

pub fn search_address(settings: &HomeSettings, query: &str) -> String {
    let (base, parameter) = match settings.search_engine {
        SearchEngine::Google => ("https://www.google.com/search", "q"),
        SearchEngine::Bing => ("https://www.bing.com/search", "q"),
        SearchEngine::Brave => ("https://search.brave.com/search", "q"),
        SearchEngine::Custom => {
            if let Some(address) = custom_search_address(&settings.custom_search_url, query) {
                return address;
            }
            ("https://duckduckgo.com/", "q")
        }
        SearchEngine::DuckDuckGo => ("https://duckduckgo.com/", "q"),
    };
    let mut url = Url::parse(base).expect("valid search URL");
    url.query_pairs_mut().append_pair(parameter, query);
    url.into()
}

pub fn home_settings(store: &crate::storage::LocalStore) -> HomeSettings {
    store
        .list_by_type(ItemType::Setting)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|record| serde_json::from_str::<SettingPayload>(&record.payload_json).ok())
        .find(|setting| setting.key == HOME_SETTING)
        .and_then(|setting| serde_json::from_str(&setting.value).ok())
        .unwrap_or_default()
}

pub fn home_address(value: &str) -> Option<String> {
    let value = value.trim();
    let url = if value.contains("://") {
        Url::parse(value).ok()?
    } else {
        if value.is_empty() || value.contains(char::is_whitespace) {
            return None;
        }
        Url::parse(&format!("https://{value}")).ok()?
    };
    let web = matches!(url.scheme(), "http" | "https") && url.has_host();
    let workspace = url.scheme() == "harbor"
        && matches!(
            url.host_str(),
            Some(
                "home"
                    | "bookmarks"
                    | "history"
                    | "logins"
                    | "adblock"
                    | "scripts"
                    | "server"
                    | "settings"
            )
        );
    (url.username().is_empty() && url.password().is_none() && (web || workspace))
        .then(|| url.into())
}

pub struct ScriptReview {
    token: Uuid,
    source_url: String,
    script: UserScript,
}

pub fn store_adblock(
    store: &crate::storage::LocalStore,
    payload: &AdblockConfigPayload,
) -> rusqlite::Result<()> {
    let id = id_for("harbor:adblock-settings");
    let rows = store.list_by_type(ItemType::AdblockConfig)?;
    let timestamp = rows
        .iter()
        .map(|row| row.updated_at.saturating_add(1))
        .max()
        .unwrap_or(0)
        .max(now_unix());
    let json = serde_json::to_string(payload)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    store.transaction(|store| {
        for row in rows.iter().filter(|row| row.id != id) {
            store.upsert_lww(row.id, row.item_type, "{}", true, true, timestamp)?;
        }
        store.upsert_lww(id, ItemType::AdblockConfig, &json, false, true, timestamp)?;
        Ok::<_, rusqlite::Error>(())
    })
}

pub fn remote_script_url(raw: &str) -> Option<Url> {
    script_source_url(raw).filter(|url| url.path().to_ascii_lowercase().ends_with(".user.js"))
}

pub fn script_source_url(raw: &str) -> Option<Url> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

fn id_for(key: &str) -> Uuid {
    let digest = hash_token(key);
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 prefix"))
}

fn script_id(source_url: &str) -> Uuid {
    let mut url = script_source_url(source_url).expect("validated script URL");
    if url.path().to_ascii_lowercase().ends_with(".user.js") {
        url.set_query(None);
    }
    id_for(&format!("harbor:remote-script:{url}"))
}

async fn download_script(
    url: Url,
) -> Result<(UserScript, Url), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let mut response = client.get(url).send().await.map_err(|error| {
        if error.is_timeout() {
            "The script server took too long to respond."
        } else {
            "Harbor could not reach the script URL over HTTPS. Check the link and its certificate."
        }
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "The script server returned HTTP {}.",
            response.status().as_u16()
        )
        .into());
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|mime| mime.starts_with("text/html"))
    {
        return Err(
            "This link returned a web page. Open the raw HTTPS .user.js file to install it.".into(),
        );
    }
    let final_url = response.url().clone();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_SCRIPT_BYTES {
            return Err("User scripts must be 256 KB or smaller.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((review(String::from_utf8(bytes)?)?, final_url))
}

#[cfg(test)]
mod tests {
    use super::home_address;

    #[test]
    fn home_destination_accepts_workspace_and_web_urls() {
        assert_eq!(
            home_address("harbor://home").as_deref(),
            Some("harbor://home")
        );
        assert_eq!(
            home_address("example.com:8443").as_deref(),
            Some("https://example.com:8443/")
        );
        assert!(home_address("harbor://chrome/").is_none());
        assert!(home_address("https://user:pass@example.com").is_none());
        assert!(home_address("javascript:alert(1)").is_none());
    }
}

fn installed_script(
    store: &crate::storage::LocalStore,
    id: Uuid,
) -> rusqlite::Result<Option<(crate::storage::LocalRecord, UserscriptPayload)>> {
    Ok(store.record(id, ItemType::Userscript)?.and_then(|record| {
        let payload = serde_json::from_str(&record.payload_json).ok()?;
        Some((record, payload))
    }))
}

fn saved_login(
    store: &crate::storage::LocalStore,
    id: Uuid,
) -> Result<LoginPayload, Box<dyn std::error::Error + Send + Sync>> {
    let record = store
        .record(id, ItemType::Login)?
        .ok_or("This login no longer exists.")?;
    Ok(serde_json::from_str(&record.payload_json)?)
}

fn script_summary(script: &UserScript) -> Value {
    json!({ "name": script.name, "version": script.version, "matches": script.matches,
        "grants": script.grants, "excludes": script.excludes, "enabled": script.enabled })
}

fn save_script(
    runtime: &HarborRuntime,
    script: UserScript,
    edit_id: Option<Uuid>,
    source_url: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let store = runtime.store.lock().expect("store lock");
    let id = edit_id
        .or_else(|| source_url.as_deref().map(script_id))
        .unwrap_or_else(Uuid::new_v4);
    let existing = installed_script(&store, id)?;
    if edit_id.is_some() && existing.is_none() {
        return Err("This script no longer exists.".into());
    }
    let enabled = existing.as_ref().is_none_or(|(_, payload)| payload.enabled);
    let source_url = source_url.or_else(|| existing.and_then(|(_, payload)| payload.source_url));
    let payload = UserscriptPayload {
        name: script.name,
        source: script.source,
        enabled,
        source_url,
    };
    store.save_record(
        id,
        ItemType::Userscript,
        &serde_json::to_string(&payload)?,
        false,
        now_unix(),
    )?;
    *runtime.scripts.lock().expect("scripts lock") = crate::load_saved_userscripts(&store);
    runtime.sync.changed();
    Ok(())
}

pub async fn handle(
    runtime: &HarborRuntime,
    route: &str,
    command: WorkspaceCommand,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let server_route = matches!(route, "setup" | "server" | "vault");
    match command {
        WorkspaceCommand::State if route == "home" => {
            let store = runtime.store.lock().expect("store lock");
            let (lists, _, _) = crate::load_saved_adblock_config(&store);
            Ok(json!({
                "bookmarks": store.count_by_type(ItemType::Bookmark)?,
                "history": store.count_by_type(ItemType::History)?,
                "scripts": store.count_by_type(ItemType::Userscript)?,
                "logins": store.count_by_type(ItemType::Login)?,
                "lists": lists.len(),
                "blocked": runtime.adblock.lock().expect("adblock lock").blocked_total(),
            }))
        }
        WorkspaceCommand::SyncStatus if route == "home" => {
            Ok(runtime.sync.status(&runtime.store).await?)
        }
        WorkspaceCommand::State if route == "bookmarks" => {
            crate::library::bookmarks(&runtime.store.lock().unwrap())
        }
        WorkspaceCommand::State if route == "history" => {
            crate::library::history(&runtime.store.lock().unwrap())
        }
        WorkspaceCommand::SaveBookmark {
            id,
            title,
            url,
            folder,
        } if route == "bookmarks" => {
            crate::library::save_bookmark(
                &runtime.store.lock().unwrap(),
                id,
                &title,
                &url,
                folder,
            )?;
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::MoveBookmark { id, folder } if route == "bookmarks" => {
            crate::library::move_bookmark(&runtime.store.lock().unwrap(), id, folder)?;
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::SaveFolder { id, name } if route == "bookmarks" => {
            crate::library::save_folder(&runtime.store.lock().unwrap(), id, &name)?;
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::DeleteFolder { id } if route == "bookmarks" => {
            crate::library::delete_folder(&runtime.store.lock().unwrap(), id)?;
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::ClearHistory { from, until } if route == "history" => {
            let removed =
                crate::library::clear_history(&runtime.store.lock().unwrap(), from, until)?;
            if removed > 0 {
                runtime.sync.changed();
            }
            Ok(json!({ "removed": removed }))
        }
        WorkspaceCommand::DeleteRecord { id }
            if matches!(route, "scripts" | "history" | "bookmarks" | "logins") =>
        {
            let kind = match route {
                "scripts" => ItemType::Userscript,
                "bookmarks" => ItemType::Bookmark,
                "logins" => ItemType::Login,
                _ => ItemType::History,
            };
            let store = runtime.store.lock().unwrap();
            crate::library::delete_record(&store, id, kind)?;
            if kind == ItemType::Userscript {
                *runtime.scripts.lock().expect("scripts lock") =
                    crate::load_saved_userscripts(&store);
            }
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::State if server_route => Ok(runtime.sync.status(&runtime.store).await?),
        WorkspaceCommand::CheckServer { server_url, pin } if server_route => {
            let address = runtime
                .sync
                .check_server(&runtime.store, server_url, pin)
                .await?;
            Ok(json!({ "server_url": address }))
        }
        WorkspaceCommand::SkipSetup if route == "setup" => {
            runtime
                .store
                .lock()
                .expect("store lock")
                .set_state("setup_complete", "1")?;
            Ok(Value::Null)
        }
        WorkspaceCommand::Pair {
            server_url,
            pin,
            device_name,
            code,
            passphrase,
        } if server_route => {
            runtime
                .sync
                .pair(
                    &runtime.store,
                    server_url,
                    pin,
                    device_name,
                    Zeroizing::new(code),
                    Zeroizing::new(passphrase),
                )
                .await?;
            Ok(Value::Null)
        }
        WorkspaceCommand::Unlock { passphrase } if server_route => {
            runtime
                .sync
                .unlock(&runtime.store, Zeroizing::new(passphrase))
                .await?;
            Ok(Value::Null)
        }
        WorkspaceCommand::SyncNow if server_route => {
            runtime.sync.sync_now(runtime).await?;
            Ok(runtime.sync.status(&runtime.store).await?)
        }
        WorkspaceCommand::State if route == "scripts" => {
            let store = runtime.store.lock().expect("store lock");
            let list = store
                .list_by_type(ItemType::Userscript)?
                .into_iter()
                .filter_map(|record| {
                    let payload: UserscriptPayload =
                        serde_json::from_str(&record.payload_json).ok()?;
                    let mut item =
                        script_summary(&UserScript::parse(payload.source, payload.enabled));
                    item["id"] = json!(record.id);
                    item["source_url"] = json!(payload.source_url);
                    Some(item)
                })
                .collect();
            Ok(Value::Array(list))
        }
        WorkspaceCommand::ScriptDetails { id } if route == "scripts" => {
            let store = runtime.store.lock().expect("store lock");
            let (_, script) =
                installed_script(&store, id)?.ok_or("This script no longer exists.")?;
            Ok(json!({ "source": script.source }))
        }
        WorkspaceCommand::SetScriptEnabled { id, enabled } if route == "scripts" => {
            let store = runtime.store.lock().expect("store lock");
            let (_, mut script) =
                installed_script(&store, id)?.ok_or("This script no longer exists.")?;
            script.enabled = enabled;
            store.save_record(
                id,
                ItemType::Userscript,
                &serde_json::to_string(&script)?,
                false,
                now_unix(),
            )?;
            *runtime.scripts.lock().expect("scripts lock") = crate::load_saved_userscripts(&store);
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::State if route == "extensions" => {
            let store = runtime.store.lock().expect("store lock");
            let active_space = runtime.tabs.lock().expect("tabs lock").active_space;
            let list = crate::extensions::list_extensions(&store);
            Ok(json!({
                "workspace": active_space,
                "extensions": list,
            }))
        }
        WorkspaceCommand::ReviewExtension { path, workspace } if route == "extensions" => {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                return Err("Enter a Chrome Web Store URL, extension ID, .crx/.zip path, or unpacked folder.".into());
            }
            let review = crate::extensions::review_extension_input(trimmed, workspace).await?;
            let summary = serde_json::to_value(&review)?;
            let mut queue = runtime
                .extension_reviews
                .lock()
                .expect("extension reviews lock");
            if queue.len() >= 16 {
                queue.pop_front();
            }
            queue.push_back(review);
            Ok(summary)
        }
        WorkspaceCommand::InstallExtension { token } if route == "extensions" => {
            let review = runtime
                .extension_reviews
                .lock()
                .expect("extension reviews lock")
                .iter()
                .find(|r| r.token == token)
                .cloned()
                .ok_or("That extension review has expired. Review the folder again.")?;
            let installed = {
                let store = runtime.store.lock().expect("store lock");
                crate::extensions::install_reviewed(&store, &runtime.profile_dir, &review)?
            };
            let store = runtime.store.lock().expect("store lock");
            *runtime.extensions.lock().expect("extensions lock") =
                crate::extensions::list_extensions(&store);
            let space = runtime.tabs.lock().expect("tabs lock").active_space;
            let exts = runtime.extensions.lock().expect("extensions lock").clone();
            let dnr =
                crate::extensions::compile_extension_dnr_rules(&exts, &runtime.profile_dir, space);
            runtime
                .adblock
                .lock()
                .expect("adblock lock")
                .set_extension_rules(&dnr);
            Ok(serde_json::to_value(installed)?)
        }
        WorkspaceCommand::SetExtensionEnabled { id, enabled } if route == "extensions" => {
            let store = runtime.store.lock().expect("store lock");
            crate::extensions::set_extension_enabled(&store, &id, enabled)?;
            *runtime.extensions.lock().expect("extensions lock") =
                crate::extensions::list_extensions(&store);
            let space = runtime.tabs.lock().expect("tabs lock").active_space;
            let exts = runtime.extensions.lock().expect("extensions lock").clone();
            let dnr =
                crate::extensions::compile_extension_dnr_rules(&exts, &runtime.profile_dir, space);
            runtime
                .adblock
                .lock()
                .expect("adblock lock")
                .set_extension_rules(&dnr);
            Ok(Value::Null)
        }
        WorkspaceCommand::RemoveExtension { id } if route == "extensions" => {
            let store = runtime.store.lock().expect("store lock");
            crate::extensions::remove_extension(&store, &runtime.profile_dir, &id)?;
            *runtime.extensions.lock().expect("extensions lock") =
                crate::extensions::list_extensions(&store);
            let space = runtime.tabs.lock().expect("tabs lock").active_space;
            let exts = runtime.extensions.lock().expect("extensions lock").clone();
            let dnr =
                crate::extensions::compile_extension_dnr_rules(&exts, &runtime.profile_dir, space);
            runtime
                .adblock
                .lock()
                .expect("adblock lock")
                .set_extension_rules(&dnr);
            Ok(Value::Null)
        }
        WorkspaceCommand::State if route == "settings" => {
            let store = runtime.store.lock().expect("store lock");
            Ok(serde_json::to_value(home_settings(&store))?)
        }
        WorkspaceCommand::SaveHomeSettings {
            show_home_button,
            home_url,
            search_engine,
            custom_search_url,
            dns_preset,
            custom_dns_url,
            background_tabs_enabled,
            background_tab_limit,
            sleep_tabs_after_minutes,
            sleep_preserve_audio,
        } if route == "settings" => {
            let home_url = home_address(&home_url)
                .ok_or("Enter an HTTP or HTTPS address or a Harbor workspace page.")?;
            let custom_search_url = custom_search_url.trim().to_string();
            if matches!(search_engine, SearchEngine::Custom)
                && custom_search_address(&custom_search_url, "harbor").is_none()
            {
                return Err(
                    "Enter an HTTPS search URL containing {query} where the search terms go."
                        .into(),
                );
            }
            let custom_dns_url = if matches!(dns_preset, DnsPreset::Custom) {
                valid_doh_url(&custom_dns_url)
                    .ok_or("Enter a valid HTTPS DNS-over-HTTPS endpoint URL.")?
            } else {
                custom_dns_url.trim().to_string()
            };
            let settings = HomeSettings {
                show_home_button,
                home_url,
                search_engine,
                custom_search_url,
                dns_preset,
                custom_dns_url,
                background_tabs_enabled,
                background_tab_limit,
                sleep_tabs_after_minutes,
                sleep_preserve_audio,
            };
            let payload = SettingPayload {
                key: HOME_SETTING.into(),
                value: serde_json::to_string(&settings)?,
            };
            let id = id_for(HOME_SETTING);
            let store = runtime.store.lock().expect("store lock");
            let updated_at = store
                .list_by_type(ItemType::Setting)?
                .into_iter()
                .find(|record| record.id == id)
                .map_or_else(now_unix, |record| {
                    now_unix().max(record.updated_at.saturating_add(1))
                });
            store.upsert_lww(
                id,
                ItemType::Setting,
                &serde_json::to_string(&payload)?,
                false,
                true,
                updated_at,
            )?;
            runtime.sync.changed();
            Ok(serde_json::to_value(settings)?)
        }
        WorkspaceCommand::ReviewScript { url } if route == "install-script" => {
            let source_url =
                script_source_url(&url).ok_or("Enter an HTTPS URL for the raw user script.")?;
            let (script, final_url) = download_script(source_url.clone()).await?;
            let token = Uuid::new_v4();
            let (action, previous_version) = {
                let store = runtime.store.lock().expect("store lock");
                match installed_script(&store, script_id(source_url.as_str()))? {
                    Some((_, previous)) => {
                        let action = if previous.source == script.source {
                            "Reinstall"
                        } else {
                            "Update"
                        };
                        (
                            action,
                            UserScript::parse(previous.source, previous.enabled).version,
                        )
                    }
                    None => ("Install", None),
                }
            };
            let data = json!({
                "source_url": source_url.as_str(), "final_url": final_url.as_str(), "source": script.source, "script": script_summary(&script),
                "previous_version": previous_version, "token": token, "action": action,
            });
            let mut pending = runtime.script_reviews.lock().expect("review lock");
            if pending.len() >= 8 {
                pending.pop_front();
            }
            pending.push_back(ScriptReview {
                token,
                source_url: source_url.into(),
                script,
            });
            Ok(data)
        }
        WorkspaceCommand::InstallScript { token } if route == "install-script" => {
            let review = {
                let mut pending = runtime.script_reviews.lock().expect("review lock");
                let index = pending
                    .iter()
                    .position(|review| review.token == token)
                    .ok_or("The script review expired. Reopen the script to review it again.")?;
                pending.remove(index).expect("active review")
            };
            save_script(runtime, review.script, None, Some(review.source_url))?;
            Ok(Value::Null)
        }
        WorkspaceCommand::SaveScript { id, source } if route == "scripts" => {
            save_script(runtime, review(source)?, id, None)?;
            Ok(Value::Null)
        }
        WorkspaceCommand::State if route == "logins" => {
            let store = runtime.store.lock().expect("store lock");
            let logins: Vec<Value> = store
                .list_by_type(ItemType::Login)?
                .into_iter()
                .filter_map(|record| {
                    let login: LoginPayload = serde_json::from_str(&record.payload_json).ok()?;
                    Some(json!({ "id": record.id, "origin": login.origin, "username": login.username }))
                })
                .collect();
            Ok(Value::Array(logins))
        }
        WorkspaceCommand::LoginPassword { id } if route == "logins" => {
            let store = runtime.store.lock().expect("store lock");
            let login = saved_login(&store, id)?;
            Ok(json!({ "password": login.password }))
        }
        WorkspaceCommand::SaveLogin {
            id,
            origin,
            username,
            password,
        } if route == "logins" => {
            let url = Url::parse(origin.trim())?;
            if !matches!(url.scheme(), "http" | "https") || !url.has_host() {
                return Err("Enter an HTTP or HTTPS website URL.".into());
            }
            let store = runtime.store.lock().expect("store lock");
            let existing = id.map(|id| saved_login(&store, id)).transpose()?;
            let (previous_password, notes) = existing
                .map(|login| (login.password, login.notes))
                .unwrap_or_default();
            let password = if password.is_empty() {
                previous_password
            } else {
                password
            };
            if password.is_empty() {
                return Err("Enter a password.".into());
            }
            let payload = LoginPayload {
                origin: url.origin().ascii_serialization(),
                username,
                password,
                notes,
            };
            store.save_record(
                id.unwrap_or_else(Uuid::new_v4),
                ItemType::Login,
                &serde_json::to_string(&payload)?,
                false,
                now_unix(),
            )?;
            runtime.sync.changed();
            Ok(Value::Null)
        }
        WorkspaceCommand::State if route == "adblock" => {
            let store = runtime.store.lock().expect("store lock");
            let (lists, rules, _) = crate::load_saved_adblock_config(&store);
            Ok(json!({ "lists": lists, "rules": rules }))
        }
        WorkspaceCommand::SaveAdblock { lists, rules } if route == "adblock" => {
            if lists.iter().any(|url| {
                Url::parse(url)
                    .ok()
                    .is_none_or(|url| url.scheme() != "https" || !url.has_host())
            }) {
                return Err("Filter list URLs must use HTTPS.".into());
            }
            let store = runtime.store.lock().expect("store lock");
            let (_, _, allowlisted_domains) = crate::load_saved_adblock_config(&store);
            let payload = AdblockConfigPayload {
                list_urls: lists,
                custom_rules: rules,
                allowlisted_domains,
            };
            store_adblock(&store, &payload)?;
            runtime.sync.changed();
            runtime.filters_changed.notify_one();
            Ok(Value::Null)
        }
        _ => Err("This action is unavailable on the current page.".into()),
    }
}
