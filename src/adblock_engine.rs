use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use adblock::Engine;
use adblock::request::Request;

pub struct AdblockManager {
    engine: Engine,
    extension_engine: Option<Engine>,
    allowlisted_domains: HashSet<String>,
    blocked_count: AtomicU64,
}

impl AdblockManager {
    pub fn from_rules(rules: &[String], allowlisted_domains: Vec<String>) -> Self {
        let engine = Engine::new_with_list_text(rules.join("\n"));
        Self {
            engine,
            extension_engine: None,
            allowlisted_domains: allowlisted_domains.into_iter().collect(),
            blocked_count: AtomicU64::new(0),
        }
    }

    pub fn set_extension_rules(&mut self, rules: &[String]) {
        self.extension_engine = if rules.is_empty() {
            None
        } else {
            Some(Engine::new_with_list_text(rules.join("\n")))
        };
    }

    /// Loads a pre-compiled binary cache from disk for instant startup.
    pub fn load_cached(cache_path: &Path, allowlisted_domains: Vec<String>) -> Option<Self> {
        let bytes = fs::read(cache_path).ok()?;
        let mut engine = Engine::default();
        engine.deserialize(&bytes).ok()?;
        Some(Self {
            engine,
            extension_engine: None,
            allowlisted_domains: allowlisted_domains.into_iter().collect(),
            blocked_count: AtomicU64::new(0),
        })
    }

    /// Saves the compiled filter engine to disk so startup skips text parsing.
    pub fn save_cache(&self, cache_path: &Path) -> std::io::Result<()> {
        let data = self.engine.serialize();
        fs::write(cache_path, data)
    }

    pub fn should_block(&self, url: &str, source_url: &str, resource_type: &str) -> bool {
        if self.is_allowlisted(source_url) {
            return false;
        }
        let Ok(request) = Request::new(url, source_url, resource_type, "GET") else {
            return false;
        };
        let matched = self.engine.check_network_request(&request).should_block()
            || self
                .extension_engine
                .as_ref()
                .is_some_and(|ext| ext.check_network_request(&request).should_block());
        if matched {
            self.blocked_count.fetch_add(1, Ordering::Relaxed);
        }
        matched
    }

    pub fn blocked_total(&self) -> u64 {
        self.blocked_count.load(Ordering::Relaxed)
    }

    pub fn replace_rules(&mut self, replacement: Self) {
        self.engine = replacement.engine;
        self.allowlisted_domains = replacement.allowlisted_domains;
    }
    pub fn set_allowlisted(&mut self, domain: &str, allow: bool) {
        if allow {
            self.allowlisted_domains.insert(domain.to_string());
        } else {
            self.allowlisted_domains.remove(domain);
        }
    }

    pub fn cosmetic_hide_css(&self, page_url: &str) -> Option<String> {
        if self.is_allowlisted(page_url) {
            return None;
        }
        let resources = self.engine.url_cosmetic_resources(page_url);
        if resources.hide_selectors.is_empty() {
            return None;
        }
        let selectors: Vec<&str> = resources
            .hide_selectors
            .iter()
            .map(|s| s.as_str())
            .collect();
        Some(format!(
            "{} {{ display: none !important; }}",
            selectors.join(", ")
        ))
    }

    pub fn is_allowlisted(&self, page_url: &str) -> bool {
        reqwest::Url::parse(page_url).ok().is_some_and(|url| {
            url.host_str()
                .is_some_and(|host| self.allowlisted_domains.contains(host))
        })
    }
}

/// Fetches remote filter lists over HTTPS, merges custom manual rules,
/// and rebuilds the serialized binary cache for instant next startup.
pub async fn refresh_remote_lists(
    list_urls: &[String],
    custom_rules: &[String],
    allowlisted_domains: Vec<String>,
    cache_path: &Path,
    store: &std::sync::Mutex<crate::storage::LocalStore>,
) -> Result<AdblockManager, reqwest::Error> {
    let http = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut all_rules: Vec<String> = Vec::new();
    for url in list_urls {
        let downloaded = async {
            let mut response = http.get(url).send().await.ok()?.error_for_status().ok()?;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            String::from_utf8(bytes).ok()
        }
        .await;
        let store = store.lock().expect("store lock");
        if let Some(rules) = downloaded {
            let _ = store.save_filter_cache(url, &rules);
            all_rules.extend(rules.lines().map(str::to_owned));
        } else if let Ok(Some(rules)) = store.filter_cache(url) {
            all_rules.extend(rules.lines().map(str::to_owned));
        }
    }
    all_rules.extend(custom_rules.iter().cloned());

    let mgr = AdblockManager::from_rules(&all_rules, allowlisted_domains);
    let _ = mgr.save_cache(cache_path);
    Ok(mgr)
}
