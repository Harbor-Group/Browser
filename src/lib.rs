mod adblock_engine;
mod autofill;
mod cef_shell;
mod extensions;
mod integrity;
mod ipc_router;
mod library;
mod reader_mode;
mod scheme_handler;
mod storage;
mod sync_worker;
mod tab_manager;
mod tls_pin;
mod userscripts;
mod workspace;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use adblock_engine::{AdblockManager, refresh_remote_lists};
use cef_shell::{BrowserShellConfig, HarborRuntime, launch_browser_shell};
use shared_protocol::types::{AdblockConfigPayload, ItemType, UserscriptPayload};
use storage::LocalStore;
use sync_worker::SyncService;
use tab_manager::TabManager;
use userscripts::UserScript;

fn load_saved_userscripts(store: &LocalStore) -> Vec<UserScript> {
    let mut loaded = Vec::new();
    if let Ok(records) = store.list_by_type(ItemType::Userscript) {
        for rec in records {
            if let Ok(payload) = serde_json::from_str::<UserscriptPayload>(&rec.payload_json) {
                loaded.push(UserScript::parse(payload.source, payload.enabled));
            }
        }
    }
    loaded
}

fn load_saved_adblock_config(store: &LocalStore) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut list_urls = Vec::new();
    let mut custom_rules = Vec::new();
    let mut allowlisted = Vec::new();
    let mut has_config = false;

    if let Ok(records) = store.list_by_type(ItemType::AdblockConfig) {
        for rec in records {
            if let Ok(cfg) = serde_json::from_str::<AdblockConfigPayload>(&rec.payload_json) {
                has_config = true;
                list_urls.extend(cfg.list_urls);
                custom_rules.extend(cfg.custom_rules);
                allowlisted.extend(cfg.allowlisted_domains);
            }
        }
    }
    if !has_config {
        custom_rules.push("||doubleclick.net^".to_string());
    }
    (list_urls, custom_rules, allowlisted)
}

async fn run_browser_app(sandbox_info: *mut u8) -> Result<(), Box<dyn std::error::Error>> {
    // Verify CEF runtime DLLs against `cef_integrity.json` before starting CEF.
    #[cfg(feature = "cef-runtime")]
    {
        if let Err(err) = integrity::verify_cef_binaries() {
            return Err(err.into());
        }
    }

    let exe_path = std::env::current_exe()?;
    let data_dir = exe_path.with_file_name("harbor_profile");
    std::fs::create_dir_all(&data_dir)?;

    let store_inner = LocalStore::open(&data_dir.join("local.db"))?;
    let homepage = if store_inner.get_state("setup_complete")?.as_deref() == Some("1") {
        "harbor://home"
    } else {
        "harbor://setup"
    };
    let initial_scripts = load_saved_userscripts(&store_inner);
    let (_, custom_rules, allowlisted) = load_saved_adblock_config(&store_inner);

    let store = Arc::new(Mutex::new(store_inner));
    let tabs = Arc::new(Mutex::new(TabManager::new(homepage)));

    // Load cached binary adblock engine for instant startup, or compile from saved rules.
    let cache_path = data_dir.join("adblock.bin");
    let mut adblock_mgr = AdblockManager::load_cached(&cache_path, allowlisted.clone())
        .unwrap_or_else(|| {
            let mgr = AdblockManager::from_rules(&custom_rules, allowlisted.clone());
            let _ = mgr.save_cache(&cache_path);
            mgr
        });
    let initial_extensions = extensions::list_extensions(&store.lock().expect("store lock"));
    let initial_space = tabs.lock().expect("tabs lock").active_space;
    let initial_dnr =
        extensions::compile_extension_dnr_rules(&initial_extensions, &data_dir, initial_space);
    adblock_mgr.set_extension_rules(&initial_dnr);
    let adblock = Arc::new(Mutex::new(adblock_mgr));
    let scripts = Arc::new(Mutex::new(initial_scripts));
    let extensions = Arc::new(Mutex::new(initial_extensions));

    let filters_changed = Arc::new(tokio::sync::Notify::new());
    {
        let store = store.clone();
        let adblock = adblock.clone();
        let changed = filters_changed.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(86_400));
            loop {
                tokio::select! {
                    _ = ticker.tick() => {},
                    _ = changed.notified() => {},
                }
                let (list_urls, custom_rules, allowlisted) =
                    load_saved_adblock_config(&store.lock().expect("store lock"));
                match refresh_remote_lists(
                    &list_urls,
                    &custom_rules,
                    allowlisted,
                    &cache_path,
                    &store,
                )
                .await
                {
                    Ok(manager) => {
                        adblock.lock().expect("adblock lock").replace_rules(manager);
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "failed to refresh remote adblock lists");
                    }
                }
            }
        });
    }

    let sync = Arc::new(SyncService::new());
    let runtime = HarborRuntime {
        tabs,
        adblock,
        scripts,
        extensions,
        store,
        sync: sync.clone(),
        tasks: tokio::runtime::Handle::current(),
        script_reviews: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        extension_reviews: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        profile_dir: data_dir.clone(),
        filters_changed,
    };
    tokio::spawn(sync.run(runtime.clone()));

    launch_browser_shell(
        BrowserShellConfig {
            profile_dir: data_dir.to_string_lossy().into_owned(),
            homepage: homepage.to_string(),
            ..Default::default()
        },
        runtime,
        sandbox_info,
    )?;

    Ok(())
}

fn start_browser(sandbox_info: *mut u8) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_browser_app(sandbox_info))
}

pub fn run_from_executable() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(all(windows, feature = "cef-runtime")) {
        return Err("CEF runtime (libcef.dll) is required next to harbor.exe.".into());
    }
    if let Some(code) = cef_shell::early_sub_process_check(std::ptr::null_mut()) {
        std::process::exit(code);
    }
    tracing_subscriber::fmt::init();
    start_browser(std::ptr::null_mut())
}

#[cfg(all(windows, feature = "cef-runtime"))]
#[unsafe(no_mangle)]
extern "C" fn RunWinMain(
    _instance: *mut std::ffi::c_void,
    _command_line: *mut u16,
    _show: i32,
    sandbox_info: *mut u8,
    _version_info: *mut std::ffi::c_void,
) -> i32 {
    if sandbox_info.is_null() {
        return 1;
    }
    if let Some(code) = cef_shell::early_sub_process_check(sandbox_info) {
        return code;
    }

    let log = std::env::current_exe()
        .and_then(|exe| std::fs::File::create(exe.with_file_name("harbor.log")));
    if let Ok(file) = log {
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init();
    }
    match start_browser(sandbox_info) {
        Ok(()) => 0,
        Err(error) => {
            tracing::error!(%error, "Harbor could not start");
            1
        }
    }
}
