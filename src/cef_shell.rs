//! CEF Browser Shell integration layer for Harbor.
//!
//! Layout contract:
//! - Top 48px: Unified 1-row Harbor chrome (`harbor://chrome`) with native OS drag regions.
//! - Below 48px: Active tab viewport (or two side-by-side panes when Split View is active).
//! - Popovers occupy a separate CEF overlay above the content viewport.

use std::sync::{Arc, Mutex};

use crate::adblock_engine::AdblockManager;
use crate::autofill::AUTOFILL_CONTENT_SCRIPT;
use crate::scheme_handler::resolve_harbor_url;
use crate::storage::LocalStore;
use crate::tab_manager::{CHROME_HEIGHT_PX, TabManager};
use crate::userscripts::UserScript;

#[derive(Clone)]
pub struct BrowserShellConfig {
    pub profile_dir: String,
    pub homepage: String,
    pub initial_width: i32,
    pub initial_height: i32,
}

impl Default for BrowserShellConfig {
    fn default() -> Self {
        Self {
            profile_dir: "./harbor_profile".to_string(),
            homepage: "harbor://home".to_string(),
            initial_width: 1280,
            initial_height: 820,
        }
    }
}

/// Shared runtime state accessible by CEF handlers (scheme, adblock, load, display, window).
#[derive(Clone)]
pub struct HarborRuntime {
    pub tabs: Arc<Mutex<TabManager>>,
    pub adblock: Arc<Mutex<AdblockManager>>,
    pub scripts: Arc<Mutex<Vec<UserScript>>>,
    pub extensions: Arc<Mutex<Vec<crate::extensions::InstalledExtension>>>,
    pub store: Arc<Mutex<LocalStore>>,
    pub sync: Arc<crate::sync_worker::SyncService>,
    pub tasks: tokio::runtime::Handle,
    pub script_reviews: Arc<Mutex<std::collections::VecDeque<crate::workspace::ScriptReview>>>,
    pub extension_reviews:
        Arc<Mutex<std::collections::VecDeque<crate::extensions::ExtensionReview>>>,
    pub profile_dir: std::path::PathBuf,
    pub filters_changed: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "cef-runtime")]
mod runtime_impl {
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use cef::{args::Args, *};

    use super::{BrowserShellConfig, CHROME_HEIGHT_PX, HarborRuntime};
    use crate::autofill::AUTOFILL_CONTENT_SCRIPT;
    use crate::ipc_router::{
        ChromePopover, HostAction, PopoverKind, chrome_update_js, dispatch_ipc,
        record_history_visit,
    };
    use crate::reader_mode::READER_MODE_TOGGLE_SCRIPT;
    use crate::scheme_handler::resolve_harbor_url;
    use crate::userscripts::RunAt;
    use crate::workspace::{WorkspaceCommand, remote_script_url};

    struct PopoverSurface {
        browser_id: Arc<AtomicI32>,
        controller: Mutex<Option<OverlayController>>,
        ready: AtomicBool,
    }

    impl PopoverSurface {
        fn new() -> Self {
            Self {
                browser_id: Arc::new(AtomicI32::new(-1)),
                controller: Mutex::new(None),
                ready: AtomicBool::new(false),
            }
        }

        fn hide(&self) {
            let controller = self.controller.lock().unwrap().clone();
            if let Some(controller) = controller {
                if controller.is_visible() != 0 {
                    controller.set_visible(0);
                }
            }
        }
    }

    #[derive(Clone)]
    struct HarborShellState {
        chrome_view: Arc<Mutex<Option<BrowserView>>>,
        tab_views: Arc<Mutex<std::collections::HashMap<u64, BrowserView>>>,
        empty_view: Arc<Mutex<Option<BrowserView>>>,
        content_panel: Arc<Mutex<Option<Panel>>>,
        chrome_browser_id: Arc<AtomicI32>,
        command_popover: Arc<PopoverSurface>,
        menu_popover: Arc<PopoverSurface>,
        popover: Arc<Mutex<Option<ChromePopover>>>,
        runtime: HarborRuntime,
    }

    impl HarborShellState {
        fn tabs_popover_is_open(&self) -> bool {
            self.popover
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|popover| matches!(popover.kind, PopoverKind::Shelf))
        }

        fn content_view_for_tab(&self, id: u64) -> Option<BrowserView> {
            self.tab_views.lock().unwrap().get(&id).cloned()
        }

        fn focused_browser(&self) -> Option<Browser> {
            let id = {
                let tabs = self.runtime.tabs.lock().unwrap();
                tabs.tab(tabs.focused_tab_id())?.id
            };
            self.content_view_for_tab(id)?.browser()
        }

        fn ensure_tab_view(&self, tab_id: u64, url: &str) -> Option<(BrowserView, bool)> {
            if tab_id == crate::tab_manager::NO_TAB {
                return None;
            }
            if let Some(existing) = self.tab_views.lock().unwrap().get(&tab_id).cloned() {
                return Some((existing, false));
            }
            let panel = self.content_panel.lock().unwrap().clone()?;
            let mut delegate = HarborContentBrowserViewDelegate::new(self.runtime.clone(), tab_id);
            let mut client = HarborClient::new(self.runtime.clone(), self.clone(), true);
            let browser_settings = BrowserSettings::default();
            let view = browser_view_create(
                Some(&mut client),
                Some(&CefString::from(url)),
                Some(&browser_settings),
                None,
                None,
                Some(&mut delegate),
            )?;
            let mut child = View::from(&view);
            panel.add_child_view(Some(&mut child));
            self.tab_views.lock().unwrap().insert(tab_id, view.clone());
            Some((view, true))
        }

        fn close_tab_view(&self, tab_id: u64) {
            let removed = self.tab_views.lock().unwrap().remove(&tab_id);
            let Some(view) = removed else { return };
            if let Some(panel) = self.content_panel.lock().unwrap().clone() {
                let mut child = View::from(&view);
                panel.remove_child_view(Some(&mut child));
            }
            if let Some(browser) = view.browser() {
                let cef_id = browser.identifier();
                self.runtime.tabs.lock().unwrap().unbind_cef_browser(cef_id);
                if let Some(host) = browser.host() {
                    host.close_browser(1);
                }
            }
        }

        fn sync_tab_views(&self) {
            let (active_id, split_id, stale_ids) = {
                let tabs = self.runtime.tabs.lock().unwrap();
                let views = self.tab_views.lock().unwrap();
                let stale: Vec<u64> = views
                    .keys()
                    .copied()
                    .filter(|&id| tabs.tab(id).is_none())
                    .collect();
                (tabs.active_tab_id, tabs.split_tab_id, stale)
            };
            for sid in stale_ids {
                self.close_tab_view(sid);
            }
            let views: Vec<(u64, BrowserView)> = self
                .tab_views
                .lock()
                .unwrap()
                .iter()
                .map(|(&id, v)| (id, v.clone()))
                .collect();
            for (tid, view) in views {
                let show = tid == active_id || split_id == Some(tid);
                if (view.is_visible() != 0) != show {
                    view.set_visible(i32::from(show));
                }
                if let Some(host) = view.browser().and_then(|b| b.host()) {
                    host.was_hidden(i32::from(!show));
                }
            }
            if let Some(empty) = self.empty_view.lock().unwrap().clone() {
                let show_empty = active_id == crate::tab_manager::NO_TAB;
                if (empty.is_visible() != 0) != show_empty {
                    empty.set_visible(i32::from(show_empty));
                }
                if let Some(host) = empty.browser().and_then(|b| b.host()) {
                    host.was_hidden(i32::from(!show_empty));
                }
            }
            if let Some(panel) = self.content_panel.lock().unwrap().clone() {
                panel.layout();
                if let Some(window) = View::from(&panel).window() {
                    window.layout();
                }
            }
        }

        fn enforce_background_limits(&self) {
            let settings = {
                let store = self.runtime.store.lock().unwrap();
                crate::workspace::home_settings(&store)
            };
            let live_ids: Vec<u64> = self.tab_views.lock().unwrap().keys().copied().collect();
            if live_ids.is_empty() {
                return;
            }
            let sleep_ids = {
                let tabs = self.runtime.tabs.lock().unwrap();
                tabs.tabs_to_sleep(
                    &live_ids,
                    std::time::Instant::now(),
                    settings.background_tabs_enabled,
                    settings.background_tab_limit,
                    settings.sleep_tabs_after_minutes,
                    settings.sleep_preserve_audio,
                )
            };
            if sleep_ids.is_empty() {
                return;
            }
            for sid in sleep_ids {
                self.close_tab_view(sid);
                self.runtime
                    .tabs
                    .lock()
                    .unwrap()
                    .set_tab_sleeping(sid, true);
            }
            self.sync_tab_views();
            self.update_toolbar(false);
        }

        fn refresh_split(&self) {
            let split_tab = self.runtime.tabs.lock().unwrap().split_tab().cloned();
            if let Some(tab) = split_tab {
                let _ = self.ensure_tab_view(tab.id, &tab.url);
            }
            self.sync_tab_views();
            self.enforce_background_limits();
        }

        fn popover_for_browser(&self, id: i32) -> Option<&PopoverSurface> {
            [self.command_popover.as_ref(), self.menu_popover.as_ref()]
                .into_iter()
                .find(|surface| surface.browser_id.load(Ordering::Relaxed) == id)
        }

        fn toolbar_frame(&self) -> Option<Frame> {
            let chrome = self.chrome_view.lock().unwrap().clone()?;
            let frame = chrome.browser()?.main_frame()?;
            (CefString::from(&frame.url()).to_string() == "harbor://chrome/").then_some(frame)
        }

        fn update_toolbar(&self, close_popover: bool) {
            let Some(frame) = self.toolbar_frame() else {
                return;
            };
            let mut script = chrome_update_js(&self.runtime);
            if close_popover {
                script.push_str("window.__harborClosePopover?.();");
            }
            frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
        }

        fn update_popover(&self) {
            let popover = self.popover.lock().unwrap().clone();
            let Some(popover) = popover else {
                self.command_popover.hide();
                self.menu_popover.hide();
                return;
            };
            let (active, inactive) = if matches!(popover.kind, PopoverKind::Command) {
                (&self.command_popover, &self.menu_popover)
            } else {
                (&self.menu_popover, &self.command_popover)
            };
            inactive.hide();
            let controller = active.controller.lock().unwrap().clone();
            let Some(controller) = controller else { return };
            if !active.ready.load(Ordering::Relaxed) {
                return;
            }
            let Some(window) = controller.window() else {
                return;
            };
            let size = window.client_area_bounds_in_screen();
            let y = popover.y.clamp(0, size.height.max(0));
            let height = popover
                .height
                .clamp(0, size.height.saturating_sub(y).max(0));
            let width = popover.width.clamp(0, size.width.max(0));
            if width == 0 || height == 0 {
                controller.set_visible(0);
                return;
            }
            controller.set_bounds(Some(&cef::Rect {
                x: popover.x.clamp(0, size.width - width),
                y,
                width,
                height,
            }));
            if let Some(frame) = controller
                .contents_view()
                .and_then(|view| view.as_browser_view())
                .and_then(|view| view.browser())
                .and_then(|browser| browser.main_frame())
            {
                if CefString::from(&frame.url()).to_string() != "harbor://chrome/popover" {
                    return;
                }
                let config = serde_json::to_string(&popover).expect("serializable popover");
                let script = format!(
                    "{}window.__harborShowPopover?.({config});",
                    chrome_update_js(&self.runtime)
                );
                frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
                if controller.is_visible() == 0 {
                    controller.set_visible(1);
                }
            }
        }
    }

    wrap_task! {
        struct ActionTask {
            action: HostAction,
            shell_state: HarborShellState,
            from_popover: bool,
        }

        impl Task {
            fn execute(&self) {
                match &self.action {
                    HostAction::LoadUrl { id, url } => {
                        if *id != crate::tab_manager::NO_TAB {
                            if let Some((cv, created)) = self.shell_state.ensure_tab_view(*id, url) {
                                if !created {
                                    if let Some(b) = cv.browser() {
                                        if let Some(f) = b.main_frame() {
                                            f.load_url(Some(&CefString::from(url.as_str())));
                                        }
                                    }
                                }
                            }
                        }
                        self.shell_state.sync_tab_views();
                        self.shell_state.enforce_background_limits();
                    }
                    HostAction::SelectTab { id, url } => {
                        let _ = self.shell_state.ensure_tab_view(*id, url);
                        self.shell_state.sync_tab_views();
                        if let Some(view) = self.shell_state.content_view_for_tab(*id) {
                            view.request_focus();
                        }
                        self.shell_state.enforce_background_limits();
                    }
                    HostAction::CloseTabView(id) => {
                        self.shell_state.close_tab_view(*id);
                        self.shell_state.sync_tab_views();
                    }
                    HostAction::FocusTab(id) => {
                        self.shell_state.sync_tab_views();
                        if let Some(view) = self.shell_state.content_view_for_tab(*id) {
                            view.request_focus();
                        }
                        self.shell_state.enforce_background_limits();
                    }
                    HostAction::RefreshSplit => self.shell_state.refresh_split(),
                    HostAction::GoBack => {
                        if let Some(browser) = self.shell_state.focused_browser() {
                            browser.go_back();
                        }
                    }
                    HostAction::GoForward => {
                        if let Some(browser) = self.shell_state.focused_browser() {
                            browser.go_forward();
                        }
                    }
                    HostAction::Reload => {
                        if let Some(browser) = self.shell_state.focused_browser() {
                            browser.reload();
                        }
                    }
                    HostAction::WindowMinimize => {
                        if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                            if let Some(w) = cv.window() {
                                w.minimize();
                            }
                        }
                    }
                    HostAction::WindowMaximize => {
                        if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                            if let Some(w) = cv.window() {
                                if w.is_maximized() != 0 {
                                    w.restore();
                                } else {
                                    w.maximize();
                                }
                            }
                        }
                    }
                    HostAction::WindowClose => {
                        if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                            if let Some(w) = cv.window() {
                                w.close();
                            }
                        }
                    }
                    HostAction::SetPopover(popover) => {
                        *self.shell_state.popover.lock().unwrap() = popover.clone();
                        self.shell_state.update_popover();
                    }
                    HostAction::ToggleReaderMode => {
                        if let Some(b) = self.shell_state.focused_browser() {
                                if let Some(f) = b.main_frame() {
                                    let cur = CefString::from(&f.url()).to_string();
                                    f.execute_java_script(
                                        Some(&CefString::from(READER_MODE_TOGGLE_SCRIPT)),
                                        Some(&CefString::from(cur.as_str())),
                                        0,
                                    );
                                }
                        }
                    }
                    HostAction::ToggleDevTools => {
                        if let Some(b) = self.shell_state.focused_browser() {
                            if let Some(host) = b.host() {
                                if host.has_dev_tools() != 0 {
                                    host.close_dev_tools();
                                } else {
                                    host.show_dev_tools(
                                        Some(&WindowInfo::default()),
                                        None,
                                        Some(&BrowserSettings::default()),
                                        None,
                                    );
                                }
                            }
                        }
                    }
                    _ => {}
                }
                if self.from_popover {
                    self.shell_state.update_toolbar(matches!(self.action, HostAction::SetPopover(None)));
                }
            }
        }
    }

    fn decode_percent(s: &str) -> String {
        let mut bytes = Vec::with_capacity(s.len());
        let mut chars = s.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                let h1 = chars.next().unwrap_or(b'0');
                let h2 = chars.next().unwrap_or(b'0');
                let hex = [h1, h2];
                if let Ok(hex_str) = std::str::from_utf8(&hex) {
                    if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                        bytes.push(val);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(h1);
                bytes.push(h2);
            } else if b == b'+' {
                bytes.push(b' ');
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    wrap_task! {
        struct DeliverScriptReview {
            frame: Frame,
            page_url: String,
            result: String,
        }

        impl Task {
            fn execute(&self) {
                if CefString::from(&self.frame.url()).to_string() == self.page_url {
                    let script = format!("window.__harborScriptReview?.({});", self.result);
                    self.frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
                }
            }
        }
    }

    fn sync_local_state_doh(profile_dir: &str, mode: &str, templates: &str) {
        let path = std::path::Path::new(profile_dir).join("Local State");
        let mut root: serde_json::Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .filter(|v: &serde_json::Value| v.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(obj) = root.as_object_mut() {
            obj.insert(
                "dns_over_https".into(),
                serde_json::json!({
                    "mode": mode,
                    "templates": templates,
                }),
            );
        }
        if let Ok(serialized) = serde_json::to_string(&root) {
            let _ = std::fs::write(&path, serialized);
        }
    }

    fn apply_dns_preferences(runtime: &HarborRuntime) {
        let settings = {
            let store = runtime.store.lock().expect("store lock");
            crate::workspace::home_settings(&store)
        };
        let (mode, templates) = crate::workspace::dns_over_https_config(&settings);
        let Some(mgr) = preference_manager_get_global() else {
            tracing::warn!("preference_manager_get_global returned None");
            return;
        };
        let set_str = |key: &str, val: &str| {
            let key_cef = CefString::from(key);
            let has = mgr.has_preference(Some(&key_cef));
            let can = mgr.can_set_preference(Some(&key_cef));
            if has == 0 || can == 0 {
                tracing::warn!(%key, has, can, "CEF DNS preference unavailable or read-only");
                return;
            }
            if let Some(mut v) = value_create() {
                v.set_string(Some(&CefString::from(val)));
                let mut err = CefString::default();
                let ok = mgr.set_preference(Some(&key_cef), Some(&mut v), Some(&mut err));
                let active = mgr
                    .preference(Some(&key_cef))
                    .map(|cur| CefString::from(&cur.string()).to_string())
                    .unwrap_or_default();
                if ok == 0 && active != val {
                    let msg = err.to_string();
                    tracing::warn!(%key, %msg, %active, "failed to set CEF DNS preference");
                } else {
                    tracing::info!(%key, %active, "CEF DNS preference active");
                }
            }
        };
        // Set templates before mode so secure mode validates against the active endpoint.
        set_str("dns_over_https.templates", &templates);
        set_str("dns_over_https.mode", mode);
    }

    wrap_task! {
        struct RefreshToolbar {
            shell_state: HarborShellState,
        }

        impl Task {
            fn execute(&self) {
                self.shell_state.update_toolbar(false);
                apply_dns_preferences(&self.shell_state.runtime);
            }
        }
    }

    fn workspace_json(
        result: Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>,
    ) -> serde_json::Value {
        match result {
            Ok(data) => serde_json::json!({ "data": data }),
            Err(error) => serde_json::json!({ "error": error.to_string() }),
        }
    }

    const WORKSPACE_MESSAGE: &str = "harbor.workspace";
    const MEDIA_MESSAGE: &str = "harbor.media";
    const CWS_INSTALL_MESSAGE: &str = "harbor.cws_install";
    const CWS_BRIDGE_SCRIPT: &str = r#"(function(){
        if (window.__harborCwsInit) return;
        window.__harborCwsInit = true;
        const findId = (el) => {
            const attrs = ['data-item-id', 'data-extension-id'];
            let cur = el;
            while (cur && cur.getAttribute) {
                for (const a of attrs) {
                    const v = cur.getAttribute(a);
                    if (v && /^[a-p]{32}$/.test(v)) return v;
                }
                cur = cur.parentElement;
            }
            const segs = location.pathname.split('/');
            for (let i = segs.length - 1; i >= 0; i--) {
                if (/^[a-p]{32}$/.test(segs[i])) return segs[i];
            }
            return null;
        };
        document.addEventListener('click', (e) => {
            const btn = e.target && e.target.closest ? e.target.closest('button, [role="button"], a') : null;
            if (!btn) return;
            const txt = (btn.innerText || btn.getAttribute('aria-label') || '').trim().toLowerCase();
            if (!txt.includes('add to chrome') && !txt.includes('add extension')) return;
            const extId = findId(btn);
            if (!extId) return;
            e.preventDefault();
            e.stopPropagation();
            e.stopImmediatePropagation();
            window.__harborInstallCws?.(extId);
        }, true);
    })();"#;
    const MEDIA_BRIDGE_SCRIPT: &str = r#"(function(){
        if (window.__harborMediaInit) return;
        window.__harborMediaInit = true;
        let last = null;
        const check = () => {
            try {
                const playing = Array.from(document.querySelectorAll('audio,video'))
                    .some(el => !el.paused && !el.ended && el.readyState > 0)
                    || (navigator.mediaSession && navigator.mediaSession.playbackState === 'playing');
                if (playing !== last) {
                    last = playing;
                    window.__harborPostMedia?.(Boolean(playing));
                }
            } catch (_) {}
        };
        for (const ev of ['play', 'playing', 'pause', 'ended', 'emptied']) {
            document.addEventListener(ev, check, true);
        }
        setInterval(check, 5000);
        check();
    })();"#;

    wrap_v8_handler! {
        struct PostMediaMessage {
            frame: Frame,
        }

        impl V8Handler {
            fn execute(
                &self, _name: Option<&CefString>, _object: Option<&mut V8Value>,
                arguments: Option<&[Option<V8Value>]>, _retval: Option<&mut Option<V8Value>>,
                _exception: Option<&mut CefString>,
            ) -> i32 {
                let Some(args) = arguments else { return 0 };
                let playing = args.first().and_then(|v| v.as_ref()).is_some_and(|v| v.bool_value() != 0);
                if let Some(mut msg) = process_message_create(Some(&CefString::from(MEDIA_MESSAGE))) {
                    if let Some(list) = msg.argument_list() {
                        list.set_bool(0, i32::from(playing));
                        self.frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
                        return 1;
                    }
                }
                0
            }
        }
    }

    wrap_v8_handler! {
        struct PostCwsInstallMessage {
            frame: Frame,
        }

        impl V8Handler {
            fn execute(
                &self, _name: Option<&CefString>, _object: Option<&mut V8Value>,
                arguments: Option<&[Option<V8Value>]>, _retval: Option<&mut Option<V8Value>>,
                _exception: Option<&mut CefString>,
            ) -> i32 {
                let Some(args) = arguments else { return 0 };
                let Some(arg) = args.first().and_then(|v| v.as_ref()) else { return 0 };
                let ext_id = CefString::from(&arg.string_value()).to_string();
                if let Some(mut msg) = process_message_create(Some(&CefString::from(CWS_INSTALL_MESSAGE))) {
                    if let Some(list) = msg.argument_list() {
                        list.set_string(0, Some(&CefString::from(ext_id.as_str())));
                        self.frame.send_process_message(ProcessId::BROWSER, Some(&mut msg));
                        return 1;
                    }
                }
                0
            }
        }
    }

    wrap_task! {
        struct SleepCheckTask {
            shell_state: HarborShellState,
        }

        impl Task {
            fn execute(&self) {
                self.shell_state.enforce_background_limits();
            }
        }
    }

    fn workspace_route(url: &reqwest::Url) -> Option<&str> {
        let host = url.host_str()?;
        (url.scheme() == "harbor"
            && matches!(url.path(), "" | "/")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(
                host,
                "home"
                    | "bookmarks"
                    | "history"
                    | "setup"
                    | "server"
                    | "vault"
                    | "scripts"
                    | "install-script"
                    | "extensions"
                    | "logins"
                    | "adblock"
                    | "settings"
            ))
        .then_some(host)
    }

    #[derive(serde::Deserialize)]
    struct WorkspaceMessage {
        id: uuid::Uuid,
        command: serde_json::Value,
    }

    wrap_task! {
        struct DeliverWorkspaceReply {
            frame: Frame,
            page_url: String,
            result: String,
        }

        impl Task {
            fn execute(&self) {
                if self.frame.is_valid() != 0 && CefString::from(&self.frame.url()).to_string() == self.page_url {
                    let js = format!("window.__harborWorkspaceReply?.({});", self.result);
                    self.frame.execute_java_script(Some(&CefString::from(js.as_str())), None, 0);
                }
            }
        }
    }

    wrap_v8_handler! {
        struct PostWorkspaceMessage { frame: Frame }

        impl V8Handler {
            fn execute(
                &self, _name: Option<&CefString>, _object: Option<&mut V8Value>,
                arguments: Option<&[Option<V8Value>]>, _retval: Option<&mut Option<V8Value>>,
                exception: Option<&mut CefString>,
            ) -> i32 {
                let argument = arguments.and_then(|args| args.first()).and_then(Option::as_ref);
                if let Some(argument) = argument.filter(|arg| arg.is_string() != 0) {
                    let raw = CefString::from(&argument.string_value());
                    if raw.to_string().len() <= 2 * 1024 * 1024 {
                        if let Some(mut message) = process_message_create(Some(&CefString::from(WORKSPACE_MESSAGE))) {
                            if let Some(args) = message.argument_list() {
                                if args.set_size(1) != 0 && args.set_string(0, Some(&raw)) != 0 {
                                    self.frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
                                    return 1;
                                }
                            }
                        }
                    }
                }
                if let Some(exception) = exception {
                    *exception = CefString::from("Harbor could not send this workspace request.");
                }
                1
            }
        }
    }

    wrap_render_process_handler! {
        struct HarborWorkspaceRenderer;

        impl RenderProcessHandler {
            fn on_context_created(
                &self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                context: Option<&mut V8Context>,
            ) {
                let (Some(frame), Some(context)) = (frame, context) else { return };
                let url = CefString::from(&frame.url()).to_string();
                if frame.is_main() == 0 {
                    return;
                }
                if reqwest::Url::parse(&url).ok().as_ref().and_then(workspace_route).is_none() {
                    if let Some(host) = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)) {
                        if host == "chromewebstore.google.com" || host == "chrome.google.com" {
                            let cws_name = CefString::from("__harborInstallCws");
                            let mut cws_handler = PostCwsInstallMessage::new(frame.clone());
                            if let (Some(global), Some(mut function)) = (
                                context.global(), v8_value_create_function(Some(&cws_name), Some(&mut cws_handler)),
                            ) {
                                let attribute = sys::cef_v8_propertyattribute_t::V8_PROPERTY_ATTRIBUTE_READONLY.into();
                                global.set_value_bykey(Some(&cws_name), Some(&mut function), attribute);
                            }
                        }
                    }
                    let name = CefString::from("__harborPostMedia");
                    let mut handler = PostMediaMessage::new(frame.clone());
                    if let (Some(global), Some(mut function)) = (
                        context.global(), v8_value_create_function(Some(&name), Some(&mut handler)),
                    ) {
                        let attribute = sys::cef_v8_propertyattribute_t::V8_PROPERTY_ATTRIBUTE_READONLY.into();
                        global.set_value_bykey(Some(&name), Some(&mut function), attribute);
                    }
                    return;
                }
                let name = CefString::from("__harborPostWorkspace");
                let mut handler = PostWorkspaceMessage::new(frame.clone());
                if let (Some(global), Some(mut function)) = (
                    context.global(), v8_value_create_function(Some(&name), Some(&mut handler)),
                ) {
                    let attribute = sys::cef_v8_propertyattribute_t::V8_PROPERTY_ATTRIBUTE_READONLY.into();
                    global.set_value_bykey(Some(&name), Some(&mut function), attribute);
                }
            }
        }
    }

    wrap_resource_handler! {
        struct HarborResourceHandler {
            body: Vec<u8>,
            mime_type: String,
            offset: Arc<AtomicUsize>,
        }

        impl ResourceHandler {
            fn open(
                &self,
                _request: Option<&mut Request>,
                handle_request: Option<&mut i32>,
                _callback: Option<&mut Callback>,
            ) -> i32 {
                if let Some(h) = handle_request {
                    *h = 1;
                }
                1
            }

            fn response_headers(
                &self,
                response: Option<&mut Response>,
                response_length: Option<&mut i64>,
                _redirect_url: Option<&mut CefString>,
            ) {
                if let Some(res) = response {
                    res.set_mime_type(Some(&CefString::from(self.mime_type.as_str())));
                    res.set_charset(Some(&CefString::from("utf-8")));
                    if self.mime_type == "application/json" {
                        res.set_header_by_name(
                            Some(&CefString::from("Cache-Control")),
                            Some(&CefString::from("no-store")),
                            1,
                        );
                    }
                    if self.mime_type == "text/html" {
                        res.set_header_by_name(
                            Some(&CefString::from("Content-Security-Policy")),
                            Some(&CefString::from("default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; worker-src 'self' blob:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; form-action 'none'; base-uri 'none'")),
                            1,
                        );
                    }
                    res.set_status(200);
                    res.set_status_text(Some(&CefString::from("OK")));
                }
                if let Some(len) = response_length {
                    *len = self.body.len() as i64;
                }
            }

            fn read(
                &self,
                data_out: *mut u8,
                bytes_to_read: i32,
                bytes_read: Option<&mut i32>,
                _callback: Option<&mut ResourceReadCallback>,
            ) -> i32 {
                let pos = self.offset.load(Ordering::Relaxed);
                let body = &self.body;
                if pos >= body.len() || data_out.is_null() || bytes_to_read <= 0 {
                    if let Some(read) = bytes_read {
                        *read = 0;
                    }
                    return 0;
                }
                let remaining = body.len() - pos;
                let to_copy = remaining.min(bytes_to_read as usize);
                unsafe {
                    std::ptr::copy_nonoverlapping(body.as_ptr().add(pos), data_out, to_copy);
                }
                self.offset.fetch_add(to_copy, Ordering::Relaxed);
                if let Some(read) = bytes_read {
                    *read = to_copy as i32;
                }
                1
            }
        }
    }

    fn resource(body: Vec<u8>, mime: String) -> ResourceHandler {
        HarborResourceHandler::new(body, mime, Arc::new(AtomicUsize::new(0)))
    }

    fn make_harbor_resource_handler(url: &str) -> ResourceHandler {
        tracing::debug!(
            route = url.split('?').next().unwrap_or(url),
            "serving Harbor page"
        );
        let (mime, body) = resolve_harbor_url(url)
            .map(|r| (r.mime_type.to_string(), r.render_body()))
            .unwrap_or_else(|| ("text/plain".to_string(), b"Harbor route not found".to_vec()));
        resource(body, mime)
    }

    wrap_scheme_handler_factory! {
        struct HarborSchemeHandlerFactory {
            runtime: HarborRuntime,
            shell_state: HarborShellState,
        }

        impl SchemeHandlerFactory {
            fn create(
                &self,
                browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                _scheme_name: Option<&CefString>,
                request: Option<&mut Request>,
            ) -> Option<ResourceHandler> {
                let request = request?;
                let url = CefString::from(&request.url()).to_string();
                if let Some((body, mime)) = crate::scheme_handler::resolve_monaco_asset(&url) {
                    return Some(resource(body.to_vec(), mime.to_string()));
                }
                if url.starts_with("harbor://extension-asset/") {
                    let extensions = self.runtime.extensions.lock().expect("extensions lock");
                    if let Some((mime, body)) = crate::extensions::resolve_extension_asset(
                        &extensions,
                        &self.runtime.profile_dir,
                        &url,
                    ) {
                        return Some(resource(body, mime.to_string()));
                    }
                    return Some(resource(b"Extension asset not found".to_vec(), "text/plain".to_string()));
                }
                if let Some(encoded) = url.strip_prefix("harbor://chrome/action?data=") {
                    let browser = browser?;
                    let frame = frame?;
                    let frame_url = CefString::from(&frame.url()).to_string();
                    let from_toolbar = browser.identifier() == self.shell_state.chrome_browser_id.load(Ordering::Relaxed)
                        && frame_url == "harbor://chrome/";
                    let from_popover = frame_url == "harbor://chrome/popover"
                        && self.shell_state.popover_for_browser(browser.identifier()).is_some();
                    if !(from_toolbar || from_popover)
                        || frame.is_main() == 0
                        || CefString::from(&request.method()).to_string() != "POST"
                        || CefString::from(&request.header_by_name(Some(&CefString::from("X-Harbor-Chrome")))).to_string() != "1"
                    {
                        return None;
                    }
                    let raw_payload = decode_percent(encoded);

                    let result = dispatch_ipc(&self.runtime, &raw_payload);
                    if from_popover || !matches!(result.host_action, HostAction::None) {
                        let mut task = ActionTask::new(result.host_action, self.shell_state.clone(), from_popover);
                        let _ = post_task(ThreadId::UI, Some(&mut task));
                    }

                    let body = serde_json::to_vec(&result.chrome_snapshot).unwrap_or_default();
                    return Some(resource(body, "application/json".to_string()));
                }

                Some(make_harbor_resource_handler(&url))
            }
        }
    }

    wrap_drag_handler! {
        struct HarborDragHandler;

        impl DragHandler {
            fn on_draggable_regions_changed(
                &self,
                browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                regions: Option<&[DraggableRegion]>,
            ) {
                if !frame.is_some_and(|frame| frame.is_main() != 0
                    && CefString::from(&frame.url()).to_string() == "harbor://chrome/") {
                    return;
                }
                if let Some(window) = browser.and_then(|browser| browser_view_get_for_browser(Some(browser)))
                    .and_then(|view| view.window()) {
                    window.set_draggable_regions(regions);
                }
            }
        }
    }

    // Intercepts network requests to run Brave's native `adblock` engine before any bytes leave the host.
    wrap_resource_request_handler! {
        struct HarborResourceRequestHandler {
            runtime: HarborRuntime,
        }

        impl ResourceRequestHandler {
            fn on_before_resource_load(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                request: Option<&mut Request>,
                _callback: Option<&mut Callback>,
            ) -> ReturnValue {
                let Some(req) = request else {
                    return ReturnValue::CONTINUE;
                };
                let url = CefString::from(&req.url()).to_string();
                if url.starts_with("harbor://") {
                    return ReturnValue::CONTINUE;
                }
                let source_url = CefString::from(&req.referrer_url()).to_string();
                let blocked = {
                    let mgr = self.runtime.adblock.lock().expect("adblock lock");
                    mgr.should_block(&url, &source_url, "other")
                };
                if blocked {
                    if let Some(b) = browser {
                        let mut tabs = self.runtime.tabs.lock().expect("tabs lock");
                        if let Some(tab_id) = tabs.tab_id_for_cef(b.identifier()) {
                            tabs.record_blocked_request(tab_id);
                        }
                    }
                    return ReturnValue::CANCEL;
                }
                ReturnValue::CONTINUE
            }
        }
    }

    wrap_request_handler! {
        struct HarborRequestHandler {
            runtime: HarborRuntime,
            is_content: bool,
        }

        impl RequestHandler {
            fn on_before_browse(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                request: Option<&mut Request>,
                _user_gesture: i32,
                _is_redirect: i32,
            ) -> i32 {
                let (Some(frame), Some(request)) = (frame, request) else { return 0 };
                if !self.is_content || frame.is_main() == 0
                    || CefString::from(&request.method()).to_string() != "GET" {
                    return 0;
                }
                open_script_review(frame, &CefString::from(&request.url()).to_string())
            }

            fn on_open_urlfrom_tab(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                target_url: Option<&CefString>,
                _target_disposition: WindowOpenDisposition,
                user_gesture: i32,
            ) -> i32 {
                if !self.is_content || user_gesture == 0 {
                    return 0;
                }
                let (Some(frame), Some(url)) = (browser.and_then(|browser| browser.main_frame()), target_url)
                    else { return 0 };
                open_script_review(&frame, &url.to_string())
            }

            fn resource_request_handler(
                &self,
                _browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _request: Option<&mut Request>,
                _is_navigation: i32,
                _is_download: i32,
                _request_initiator: Option<&CefString>,
                _disable_default_handling: Option<&mut i32>,
            ) -> Option<ResourceRequestHandler> {
                Some(HarborResourceRequestHandler::new(self.runtime.clone()))
            }
        }
    }

    wrap_task! {
        struct OpenScriptReview {
            frame: Frame,
            url: String,
        }

        impl Task {
            fn execute(&self) {
                self.frame.load_url(Some(&CefString::from(self.url.as_str())));
            }
        }
    }

    fn open_script_review(frame: &Frame, raw: &str) -> i32 {
        let Some(url) = remote_script_url(raw) else {
            return 0;
        };
        open_script_review_url(frame, &url)
    }

    fn open_script_review_url(frame: &Frame, url: &reqwest::Url) -> i32 {
        let mut review = reqwest::Url::parse("harbor://install-script/").expect("valid Harbor URL");
        review.query_pairs_mut().append_pair("url", url.as_str());
        let mut task = OpenScriptReview::new(frame.clone(), review.into());
        post_task(ThreadId::UI, Some(&mut task))
    }

    wrap_download_handler! {
        struct HarborUserscriptDownloadHandler;

        impl DownloadHandler {
            fn can_download(
                &self, _browser: Option<&mut Browser>, _url: Option<&CefString>,
                _request_method: Option<&CefString>,
            ) -> i32 {
                1
            }

            fn on_before_download(
                &self, browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
                suggested_name: Option<&CefString>, _callback: Option<&mut BeforeDownloadCallback>,
            ) -> i32 {
                let (Some(item), Some(frame)) = (item, browser.and_then(|browser| browser.main_frame()))
                    else { return 0 };
                let raw = CefString::from(&item.url()).to_string();
                let url = remote_script_url(&raw).or_else(|| {
                    suggested_name
                        .filter(|name| name.to_string().to_ascii_lowercase().ends_with(".user.js"))
                        .and_then(|_| crate::workspace::script_source_url(&raw))
                });
                url.map_or(0, |url| open_script_review_url(&frame, &url))
            }
        }
    }

    wrap_life_span_handler! {
        struct HarborContentLifeSpan;

        impl LifeSpanHandler {
            fn on_before_popup(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _popup_id: i32,
                target_url: Option<&CefString>,
                _target_frame_name: Option<&CefString>,
                _target_disposition: WindowOpenDisposition,
                user_gesture: i32,
                _popup_features: Option<&PopupFeatures>,
                _window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>,
                _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<DictionaryValue>>,
                _no_javascript_access: Option<&mut i32>,
            ) -> i32 {
                if user_gesture == 0 {
                    return 0;
                }
                let (Some(frame), Some(url)) = (browser.and_then(|browser| browser.main_frame()), target_url)
                    else { return 0 };
                open_script_review(&frame, &url.to_string())
            }
        }
    }

    fn run_user_scripts(runtime: &HarborRuntime, frame: &Frame, url: &str, early: bool) {
        let scripts = runtime
            .scripts
            .lock()
            .expect("scripts lock")
            .iter()
            .filter(|script| {
                script.matches_url(url) && (script.run_at == RunAt::DocumentStart) == early
            })
            .map(|script| script.wrapped_for_injection())
            .collect::<Vec<_>>();
        let workspace = runtime.tabs.lock().expect("tabs lock").active_space;
        let ext_scripts = {
            let exts = runtime.extensions.lock().expect("extensions lock");
            crate::extensions::matching_content_scripts(
                &exts,
                &runtime.profile_dir,
                workspace,
                url,
                early,
            )
        };
        let origin = CefString::from(url);
        for script in scripts.into_iter().chain(ext_scripts) {
            frame.execute_java_script(Some(&CefString::from(script.as_str())), Some(&origin), 0);
        }
    }

    // Injects cosmetic adblock CSS, matching UserScripts, and the password autofill detector.
    wrap_load_handler! {
        struct HarborLoadHandler {
            runtime: HarborRuntime,
            shell_state: HarborShellState,
        }

        impl LoadHandler {
            fn on_load_start(
                &self,
                _browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                _transition_type: TransitionType,
            ) {
                let Some(frame) = frame.filter(|frame| frame.is_main() != 0) else { return };
                let url = CefString::from(&frame.url()).to_string();
                if url.starts_with("https://") || url.starts_with("http://") {
                    run_user_scripts(&self.runtime, frame, &url, true);
                }
            }

            fn on_loading_state_change(
                &self,
                browser: Option<&mut Browser>,
                _is_loading: i32,
                can_go_back: i32,
                can_go_forward: i32,
            ) {
                let Some(b) = browser else { return };
                if b.identifier() == self.shell_state.chrome_browser_id.load(Ordering::Relaxed)
                    || self.shell_state.popover_for_browser(b.identifier()).is_some()
                {
                    return;
                }
                {
                    let mut tabs = self.runtime.tabs.lock().expect("tabs lock");
                    if let Some(tab_id) = tabs.tab_id_for_cef(b.identifier()) {
                        tabs.update_tab_history(tab_id, can_go_back != 0, can_go_forward != 0);
                    }
                }
                self.shell_state.update_toolbar(false);
            }

            fn on_load_end(
                &self,
                browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                _http_status_code: i32,
            ) {
                let Some(frame) = frame else { return };
                if frame.is_main() == 0 {
                    return;
                }
                let url = CefString::from(&frame.url()).to_string();
                if url == "harbor://chrome/popover" {
                    if let Some(surface) = browser.as_ref()
                        .and_then(|browser| self.shell_state.popover_for_browser(browser.identifier())) {
                        surface.ready.store(true, Ordering::Relaxed);
                        self.shell_state.update_popover();
                        return;
                    }
                }
                if url.starts_with("harbor://chrome") {
                    let update_js = chrome_update_js(&self.runtime);
                    frame.execute_java_script(
                        Some(&CefString::from(update_js.as_str())),
                        Some(&CefString::from("harbor://chrome")),
                        0,
                    );
                    return;
                }
                if url != "about:blank"
                    && reqwest::Url::parse(&url).ok().as_ref().and_then(workspace_route).is_none()
                    && url != "harbor://empty/"
                {
                    frame.execute_java_script(
                        Some(&CefString::from(MEDIA_BRIDGE_SCRIPT)),
                        Some(&CefString::from(url.as_str())),
                        0,
                    );
                    if let Some(host) = reqwest::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)) {
                        if host == "chromewebstore.google.com" || host == "chrome.google.com" {
                            frame.execute_java_script(
                                Some(&CefString::from(CWS_BRIDGE_SCRIPT)),
                                Some(&CefString::from(url.as_str())),
                                0,
                            );
                        }
                    }
                }

                let browser_id = browser.as_ref().map(|browser| browser.identifier());
                if let Some(b) = browser {
                    let can_back = b.can_go_back() != 0;
                    let can_fwd = b.can_go_forward() != 0;
                    let mut tabs = self.runtime.tabs.lock().expect("tabs lock");
                    if let Some(tab_id) = tabs.tab_id_for_cef(b.identifier()) {
                        tabs.update_tab_navigation(tab_id, Some(url.clone()), None, Some(100));
                        tabs.update_tab_history(tab_id, can_back, can_fwd);
                    }
                    drop(tabs);

                    let update_js = chrome_update_js(&self.runtime);
                    if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                        if let Some(cb) = cv.browser() {
                            if let Some(cf) = cb.main_frame() {
                                cf.execute_java_script(
                                    Some(&CefString::from(update_js.as_str())),
                                    Some(&CefString::from("harbor://chrome")),
                                    0,
                                );
                            }
                        }
                    }
                }

                if let Ok(page) = reqwest::Url::parse(&url) {
                    if page.scheme() == "harbor" && page.host_str() == Some("install-script") {
                        let source = page.query_pairs().find(|(name, _)| name == "url")
                            .map(|(_, value)| value.into_owned()).unwrap_or_default();
                        let frame = frame.clone();
                        let runtime = self.runtime.clone();
                        self.runtime.tasks.spawn(async move {
                            let result = crate::workspace::handle(
                                &runtime, "install-script", WorkspaceCommand::ReviewScript { url: source },
                            ).await;
                            let mut task = DeliverScriptReview::new(frame, url, workspace_json(result).to_string());
                            let _ = post_task(ThreadId::UI, Some(&mut task));
                        });
                        return;
                    }
                }

                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return;
                }

                // I inject cosmetic adblock CSS before running page scripts.
                if let Some(css) = self
                    .runtime
                    .adblock
                    .lock()
                    .expect("adblock lock")
                    .cosmetic_hide_css(&url)
                {
                    let js = format!(
                        "(function(){{const s=document.createElement('style');s.textContent={:?};document.head&&document.head.appendChild(s);}})();",
                        css
                    );
                    frame.execute_java_script(
                        Some(&CefString::from(js.as_str())),
                        Some(&CefString::from(url.as_str())),
                        0,
                    );
                }

                // I run matching user scripts in the page context.
                run_user_scripts(&self.runtime, frame, &url, false);

                // I only inject the password form detector on HTTPS pages.
                if url.starts_with("https://") {
                    frame.execute_java_script(
                        Some(&CefString::from(AUTOFILL_CONTENT_SCRIPT)),
                        Some(&CefString::from(url.as_str())),
                        0,
                    );
                }

                // I record the visit in encrypted local history for sync.
                let title = {
                    let tabs = self.runtime.tabs.lock().expect("tabs lock");
                    browser_id.and_then(|id| tabs.tab_id_for_cef(id))
                        .and_then(|id| tabs.tab(id))
                        .map(|tab| tab.title.clone())
                        .unwrap_or_else(|| url.clone())
                };
                record_history_visit(&self.runtime, &url, &title);
            }
        }
    }

    wrap_display_handler! {
        struct HarborDisplayHandler {
            runtime: HarborRuntime,
            shell_state: HarborShellState,
        }

        impl DisplayHandler {
            fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
                if let (Some(b), Some(t)) = (browser, title) {
                    if b.identifier() == self.shell_state.chrome_browser_id.load(Ordering::Relaxed)
                        || self.shell_state.popover_for_browser(b.identifier()).is_some() {
                        return;
                    }
                    let mut tabs = self.runtime.tabs.lock().expect("tabs lock");
                    if let Some(tab_id) = tabs.tab_id_for_cef(b.identifier()) {
                        tabs.update_tab_navigation(tab_id, None, Some(t.to_string()), None);
                    }
                    drop(tabs);

                    let update_js = chrome_update_js(&self.runtime);
                    if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                        if let Some(cb) = cv.browser() {
                            if let Some(cf) = cb.main_frame() {
                                cf.execute_java_script(
                                    Some(&CefString::from(update_js.as_str())),
                                    Some(&CefString::from("harbor://chrome")),
                                    0,
                                );
                            }
                        }
                    }
                }
            }

            fn on_address_change(
                &self,
                browser: Option<&mut Browser>,
                frame: Option<&mut Frame>,
                url: Option<&CefString>,
            ) {
                if frame.as_ref().is_some_and(|f| f.is_main() == 0) {
                    return;
                }
                if let (Some(b), Some(u)) = (browser, url) {
                    let u_str = u.to_string();
                    if u_str.starts_with("harbor://chrome") {
                        return;
                    }
                    let mut tabs = self.runtime.tabs.lock().expect("tabs lock");
                    if let Some(tab_id) = tabs.tab_id_for_cef(b.identifier()) {
                        tabs.update_tab_navigation(tab_id, Some(u_str), None, None);
                    }
                    drop(tabs);

                    let update_js = chrome_update_js(&self.runtime);
                    if let Some(cv) = self.shell_state.chrome_view.lock().ok().and_then(|g| g.as_ref().cloned()) {
                        if let Some(cb) = cv.browser() {
                            if let Some(cf) = cb.main_frame() {
                                cf.execute_java_script(
                                    Some(&CefString::from(update_js.as_str())),
                                    Some(&CefString::from("harbor://chrome")),
                                    0,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    wrap_focus_handler! {
        struct HarborContentFocusHandler {
            shell_state: HarborShellState,
        }

        impl FocusHandler {
            fn on_set_focus(&self, _browser: Option<&mut Browser>, source: FocusSource) -> i32 {
                // Switching workspaces loads the selected page while the user is still in Tabs.
                i32::from(source == FocusSource::NAVIGATION && self.shell_state.tabs_popover_is_open())
            }

            fn on_got_focus(&self, browser: Option<&mut Browser>) {
                if let Some(browser) = browser {
                    self.shell_state.runtime.tabs.lock().unwrap().focus_cef_browser(browser.identifier());
                    self.shell_state.update_toolbar(false);
                }
                if let Some(frame) = self.shell_state.toolbar_frame() {
                    frame.execute_java_script(
                        Some(&CefString::from("if (!document.hasFocus()) window.__harborDismissForPageFocus?.();")),
                        None,
                        0,
                    );
                }
            }
        }
    }

    wrap_client! {
        struct HarborClient {
            runtime: HarborRuntime,
            shell_state: HarborShellState,
            is_content: bool,
        }

        impl Client {
            fn on_process_message_received(
                &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                source_process: ProcessId, message: Option<&mut ProcessMessage>,
            ) -> i32 {
                let (Some(browser), Some(frame), Some(message)) = (browser, frame, message)
                    else { return 0 };
                let msg_name = CefString::from(&message.name()).to_string();
                if self.is_content && source_process == ProcessId::RENDERER && frame.is_main() != 0 && msg_name == CWS_INSTALL_MESSAGE {
                    if let Some(args) = message.argument_list() {
                        let ext_id = CefString::from(&args.string(0)).to_string();
                        if ext_id.len() == 32 && ext_id.bytes().all(|b| matches!(b, b'a'..=b'p')) {
                            let target = format!("harbor://extensions?install={ext_id}");
                            frame.load_url(Some(&CefString::from(target.as_str())));
                            return 1;
                        }
                    }
                    return 0;
                }
                if self.is_content && source_process == ProcessId::RENDERER && frame.is_main() != 0 && msg_name == MEDIA_MESSAGE {
                    if let Some(args) = message.argument_list() {
                        let playing = args.bool(0) != 0;
                        if self.runtime.tabs.lock().unwrap().set_tab_audible(browser.identifier(), playing) {
                            self.shell_state.update_toolbar(false);
                        }
                    }
                    return 1;
                }
                if !self.is_content || source_process != ProcessId::RENDERER || frame.is_main() == 0
                    || msg_name != WORKSPACE_MESSAGE
                    || self.runtime.tabs.lock().unwrap().tab_id_for_cef(browser.identifier()).is_none() {
                    return 0;
                }
                let page_url = CefString::from(&frame.url()).to_string();
                let Some(page) = reqwest::Url::parse(&page_url).ok() else { return 0 };
                let Some(route) = workspace_route(&page).map(str::to_owned) else { return 0 };
                let Some(args) = message.argument_list() else { return 0 };
                let raw = zeroize::Zeroizing::new(CefString::from(&args.string(0)).to_string());
                if raw.len() > 2 * 1024 * 1024 {
                    return 0;
                }
                let Ok(request) = serde_json::from_str::<WorkspaceMessage>(&raw) else { return 0 };
                let shell_state = self.shell_state.clone();
                let frame = frame.clone();
                self.runtime.tasks.spawn(async move {
                    let (refresh, result) = match serde_json::from_value::<WorkspaceCommand>(request.command) {
                        Ok(command) => {
                            let refresh = matches!(route.as_str(), "settings" | "bookmarks")
                                && !matches!(command, WorkspaceCommand::State);
                            (refresh, crate::workspace::handle(&shell_state.runtime, &route, command).await)
                        }
                        Err(_) => (false, Err("Invalid workspace request.".into())),
                    };
                    if refresh && result.is_ok() {
                        let mut task = RefreshToolbar::new(shell_state);
                        let _ = post_task(ThreadId::UI, Some(&mut task));
                    }
                    let result = serde_json::json!({ "id": request.id, "result": workspace_json(result) });
                    let mut task = DeliverWorkspaceReply::new(frame, page_url, result.to_string());
                    let _ = post_task(ThreadId::UI, Some(&mut task));
                });
                1
            }

            fn request_handler(&self) -> Option<RequestHandler> {
                Some(HarborRequestHandler::new(self.runtime.clone(), self.is_content))
            }

            fn load_handler(&self) -> Option<LoadHandler> {
                Some(HarborLoadHandler::new(self.runtime.clone(), self.shell_state.clone()))
            }

            fn display_handler(&self) -> Option<DisplayHandler> {
                Some(HarborDisplayHandler::new(self.runtime.clone(), self.shell_state.clone()))
            }

            fn drag_handler(&self) -> Option<DragHandler> {
                Some(HarborDragHandler::new())
            }

            fn focus_handler(&self) -> Option<FocusHandler> {
                self.is_content.then(|| HarborContentFocusHandler::new(self.shell_state.clone()))
            }

            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                self.is_content.then(HarborContentLifeSpan::new)
            }

            fn download_handler(&self) -> Option<DownloadHandler> {
                self.is_content.then(HarborUserscriptDownloadHandler::new)
            }
        }
    }

    wrap_browser_view_delegate! {
        struct HarborChromeBrowserViewDelegate {
            preferred_height: i32,
            browser_id: Arc<AtomicI32>,
        }

        impl ViewDelegate {
            fn preferred_size(&self, _view: Option<&mut View>) -> Size {
                Size {
                    // CEF treats a zero dimension as an unspecified preferred size.
                    width: 1,
                    height: self.preferred_height,
                }
            }
        }

        impl BrowserViewDelegate {
            fn browser_runtime_style(&self) -> RuntimeStyle {
                RuntimeStyle::ALLOY
            }

            fn on_browser_created(
                &self,
                _browser_view: Option<&mut BrowserView>,
                browser: Option<&mut Browser>,
            ) {
                if let Some(browser) = browser {
                    self.browser_id.store(browser.identifier(), Ordering::Relaxed);
                }
            }
        }
    }

    wrap_browser_view_delegate! {
        struct HarborContentBrowserViewDelegate {
            runtime: HarborRuntime,
            tab_id: u64,
        }

        impl ViewDelegate {
            fn preferred_size(&self, _view: Option<&mut View>) -> Size {
                Size { width: 1, height: 1 }
            }
        }

        impl BrowserViewDelegate {
            fn browser_runtime_style(&self) -> RuntimeStyle {
                RuntimeStyle::ALLOY
            }

            fn on_browser_created(
                &self,
                _browser_view: Option<&mut BrowserView>,
                browser: Option<&mut Browser>,
            ) {
                if let Some(b) = browser.filter(|_| self.tab_id != crate::tab_manager::NO_TAB) {
                    let cef_id = b.identifier();
                    self.runtime.tabs.lock().expect("tabs lock").bind_cef_browser(self.tab_id, cef_id);
                }
            }
        }
    }

    wrap_window_delegate! {
        struct HarborWindowDelegate {
            chrome_view: BrowserView,
            content_view: BrowserView,
            empty_view: BrowserView,
            command_view: BrowserView,
            menu_view: BrowserView,
            shell_state: HarborShellState,
            config: BrowserShellConfig,
        }

        impl ViewDelegate {}

        impl PanelDelegate {}

        impl WindowDelegate {
            fn is_frameless(&self, _window: Option<&mut Window>) -> i32 {
                1
            }

            fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
                1
            }

            fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
                1
            }

            fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
                1
            }

            fn can_close(&self, _window: Option<&mut Window>) -> i32 {
                1
            }

            fn with_standard_window_buttons(&self, _window: Option<&mut Window>) -> i32 {
                0
            }

            fn window_runtime_style(&self) -> RuntimeStyle {
                RuntimeStyle::ALLOY
            }

            fn initial_bounds(&self, _window: Option<&mut Window>) -> cef::Rect {
                cef::Rect {
                    x: 80,
                    y: 60,
                    width: self.config.initial_width,
                    height: self.config.initial_height,
                }
            }

            fn on_window_created(&self, window: Option<&mut Window>) {
                let Some(win) = window else {
                    tracing::error!("on_window_created: window is None!");
                    return;
                };
                win.set_title(Some(&CefString::from("Harbor Browser")));
                if let Some(mut icon) = image_create() {
                    let png_bytes = include_bytes!("../assets/harbor-128.png");
                    icon.add_png(1.0, Some(png_bytes));
                    win.set_window_icon(Some(&mut icon));
                    win.set_window_app_icon(Some(&mut icon));
                }

                let win_layout_settings = BoxLayoutSettings {
                    horizontal: 0,
                    main_axis_alignment: AxisAlignment::START,
                    cross_axis_alignment: AxisAlignment::STRETCH,
                    ..Default::default()
                };
                let win_box_layout = win.set_to_box_layout(Some(&win_layout_settings));

                let mut chrome_v = View::from(&self.chrome_view);
                let mut content_v = View::from(&self.content_view);
                let mut empty_v = View::from(&self.empty_view);
                empty_v.set_visible(0);

                let panel = panel_create(None).expect("create browser content panel");
                let mut panel_view = View::from(&panel);
                let content_layout = BoxLayoutSettings {
                    horizontal: 1,
                    cross_axis_alignment: AxisAlignment::STRETCH,
                    default_flex: 1,
                    ..Default::default()
                };
                panel.set_to_box_layout(Some(&content_layout));
                panel.add_child_view(Some(&mut content_v));
                panel.add_child_view(Some(&mut empty_v));
                *self.shell_state.content_panel.lock().unwrap() = Some(panel);

                win.add_child_view(Some(&mut chrome_v));
                win.add_child_view(Some(&mut panel_view));

                if let Some(wl) = win_box_layout {
                    wl.set_flex_for_view(Some(&mut chrome_v), 0);
                    wl.set_flex_for_view(Some(&mut panel_view), 1);
                }
                // The suggestions receive clicks without activating their widget.
                // The editable address field remains focused in the main toolbar.
                let mut command = View::from(&self.command_view);
                let controller = win.add_overlay_view(Some(&mut command), DockingMode::CUSTOM, 0);
                *self.shell_state.command_popover.controller.lock().unwrap() = controller;

                let mut menu = View::from(&self.menu_view);
                let controller = win.add_overlay_view(Some(&mut menu), DockingMode::CUSTOM, 1);
                *self.shell_state.menu_popover.controller.lock().unwrap() = controller;

                for key in [b'K', b'L', b'R', b'T'] {
                    win.set_accelerator(i32::from(key), i32::from(key), 0, 1, 0, 1);
                }
                // I bind F12 and Ctrl+Shift+I so I can toggle DevTools.
                win.set_accelerator(0x7B, 0x7B, 0, 0, 0, 1);
                win.set_accelerator(0x1049, i32::from(b'I'), 1, 1, 0, 1);
                win.layout();
                win.show();
                content_v.request_focus();
                tracing::info!("Harbor frameless window shown with Alloy multi-view layout");
            }

            fn on_accelerator(&self, _window: Option<&mut Window>, command_id: i32) -> i32 {
                let script = match command_id {
                    0x7B | 0x1049 => "window.__harborShortcut?.('toggle_dev_tools')",
                    _ => match u8::try_from(command_id) {
                        Ok(b'K' | b'L') => "window.__harborShortcut?.('command')",
                        Ok(b'R') => "window.__harborShortcut?.('nav_reload')",
                        Ok(b'T') => "window.__harborShortcut?.('tab_new')",
                        _ => return 0,
                    },
                };
                let Some(frame) = self.chrome_view.browser().and_then(|browser| browser.main_frame()) else {
                    return 0;
                };
                if CefString::from(&frame.url()).to_string() != "harbor://chrome/" {
                    return 0;
                }
                if matches!(u8::try_from(command_id), Ok(b'K' | b'L')) {
                    self.chrome_view.request_focus();
                }
                frame.execute_java_script(
                    Some(&CefString::from(script)),
                    Some(&CefString::from("harbor://chrome/")),
                    0,
                );
                1
            }

            fn on_window_destroyed(&self, _window: Option<&mut Window>) {
                self.shell_state.command_popover.controller.lock().unwrap().take();
                self.shell_state.menu_popover.controller.lock().unwrap().take();
                quit_message_loop();
            }
        }
    }

    wrap_browser_process_handler! {
        struct HarborBrowserProcessHandler {
            config: BrowserShellConfig,
            runtime: HarborRuntime,
        }

        impl BrowserProcessHandler {
            fn on_context_initialized(&self) {
                tracing::info!("HarborBrowserProcessHandler::on_context_initialized entered");
                let chrome_browser_id = Arc::new(AtomicI32::new(-1));
                let shell_state = HarborShellState {
                    chrome_view: Arc::new(Mutex::new(None)),
                    tab_views: Arc::new(Mutex::new(std::collections::HashMap::new())),
                    empty_view: Arc::new(Mutex::new(None)),
                    content_panel: Arc::new(Mutex::new(None)),
                    chrome_browser_id: chrome_browser_id.clone(),
                    command_popover: Arc::new(PopoverSurface::new()),
                    menu_popover: Arc::new(PopoverSurface::new()),
                    popover: Arc::new(Mutex::new(None)),
                    runtime: self.runtime.clone(),
                };

                let mut factory = HarborSchemeHandlerFactory::new(self.runtime.clone(), shell_state.clone());
                register_scheme_handler_factory(
                    Some(&CefString::from("harbor")),
                    None,
                    Some(&mut factory),
                );
                tracing::info!("Registered harbor:// scheme handler factory");
                apply_dns_preferences(&self.runtime);

                let browser_settings = BrowserSettings::default();

                let mut chrome_delegate = HarborChromeBrowserViewDelegate::new(CHROME_HEIGHT_PX, chrome_browser_id);
                let mut chrome_client = HarborClient::new(self.runtime.clone(), shell_state.clone(), false);
                let chrome_view = browser_view_create(
                    Some(&mut chrome_client),
                    Some(&CefString::from("harbor://chrome")),
                    Some(&browser_settings),
                    None,
                    None,
                    Some(&mut chrome_delegate),
                )
                .expect("create chrome browser view");

                let create_popover_view = |surface: &PopoverSurface| {
                    let mut delegate = HarborChromeBrowserViewDelegate::new(1, surface.browser_id.clone());
                    let mut client = HarborClient::new(self.runtime.clone(), shell_state.clone(), false);
                    browser_view_create(
                        Some(&mut client),
                        Some(&CefString::from("harbor://chrome/popover")),
                        Some(&browser_settings),
                        None,
                        None,
                        Some(&mut delegate),
                    ).expect("create popover browser view")
                };
                let command_view = create_popover_view(&shell_state.command_popover);
                let menu_view = create_popover_view(&shell_state.menu_popover);

                let initial_tab_id = self.runtime.tabs.lock().unwrap().active_tab_id;
                let mut content_delegate = HarborContentBrowserViewDelegate::new(self.runtime.clone(), initial_tab_id);
                let mut content_client = HarborClient::new(self.runtime.clone(), shell_state.clone(), true);
                let content_view = browser_view_create(
                    Some(&mut content_client),
                    Some(&CefString::from(self.config.homepage.as_str())),
                    Some(&browser_settings),
                    None,
                    None,
                    Some(&mut content_delegate),
                )
                .expect("create content browser view");

                let mut empty_delegate = HarborContentBrowserViewDelegate::new(self.runtime.clone(), crate::tab_manager::NO_TAB);
                let mut empty_client = HarborClient::new(self.runtime.clone(), shell_state.clone(), true);
                let empty_view = browser_view_create(
                    Some(&mut empty_client),
                    Some(&CefString::from(crate::tab_manager::EMPTY_WORKSPACE)),
                    Some(&browser_settings),
                    None,
                    None,
                    Some(&mut empty_delegate),
                ).expect("create empty workspace browser view");

                *shell_state.chrome_view.lock().unwrap() = Some(chrome_view.clone());
                shell_state.tab_views.lock().unwrap().insert(initial_tab_id, content_view.clone());
                *shell_state.empty_view.lock().unwrap() = Some(empty_view.clone());

                let timer_state = shell_state.clone();
                self.runtime.tasks.spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        let mut task = SleepCheckTask::new(timer_state.clone());
                        let _ = post_task(ThreadId::UI, Some(&mut task));
                    }
                });

                let mut delegate = HarborWindowDelegate::new(
                    chrome_view,
                    content_view,
                    empty_view,
                    command_view,
                    menu_view,
                    shell_state,
                    BrowserShellConfig {
                        profile_dir: self.config.profile_dir.clone(),
                        homepage: self.config.homepage.clone(),
                        initial_width: self.config.initial_width,
                        initial_height: self.config.initial_height,
                    },
                );
                let _ = window_create_top_level(Some(&mut delegate));
            }
        }
    }

    wrap_app! {
        struct HarborApp {
            config: BrowserShellConfig,
            runtime: HarborRuntime,
        }

        impl App {
            fn render_process_handler(&self) -> Option<RenderProcessHandler> {
                Some(HarborWorkspaceRenderer::new())
            }
            fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
                if let Some(reg) = registrar {
                    let flags = SchemeOptions::STANDARD.get_raw()
                        | SchemeOptions::SECURE.get_raw()
                        | SchemeOptions::CORS_ENABLED.get_raw()
                        | SchemeOptions::FETCH_ENABLED.get_raw();
                    reg.add_custom_scheme(Some(&CefString::from("harbor")), flags);
                }
            }

            fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
                Some(HarborBrowserProcessHandler::new(
                    BrowserShellConfig {
                        profile_dir: self.config.profile_dir.clone(),
                        homepage: self.config.homepage.clone(),
                        initial_width: self.config.initial_width,
                        initial_height: self.config.initial_height,
                    },
                    self.runtime.clone(),
                ))
            }
        }
    }

    wrap_app! {
        struct HarborSubprocessApp;

        impl App {
            fn render_process_handler(&self) -> Option<RenderProcessHandler> {
                Some(HarborWorkspaceRenderer::new())
            }
            fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
                if let Some(reg) = registrar {
                    let flags = SchemeOptions::STANDARD.get_raw()
                        | SchemeOptions::SECURE.get_raw()
                        | SchemeOptions::CORS_ENABLED.get_raw()
                        | SchemeOptions::FETCH_ENABLED.get_raw();
                    reg.add_custom_scheme(Some(&CefString::from("harbor")), flags);
                }
            }
        }
    }

    pub fn early_sub_process_check(sandbox_info: *mut u8) -> Option<i32> {
        if !std::env::args_os().any(|arg| arg.to_string_lossy().starts_with("--type=")) {
            return None;
        }

        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

        let main_args = Args::new();
        let mut app = HarborSubprocessApp::new();
        let exit_code =
            execute_process(Some(main_args.as_main_args()), Some(&mut app), sandbox_info);
        Some(if exit_code >= 0 { exit_code } else { 1 })
    }

    pub fn run(
        config: BrowserShellConfig,
        runtime: HarborRuntime,
        sandbox_info: *mut u8,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let args = Args::new();
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

        {
            let store = runtime.store.lock().expect("store lock");
            let home = crate::workspace::home_settings(&store);
            let (mode, templates) = crate::workspace::dns_over_https_config(&home);
            sync_local_state_doh(&config.profile_dir, mode, &templates);
        }

        let mut app = HarborApp::new(
            BrowserShellConfig {
                profile_dir: config.profile_dir.clone(),
                homepage: config.homepage.clone(),
                initial_width: config.initial_width,
                initial_height: config.initial_height,
            },
            runtime,
        );

        let settings = Settings {
            root_cache_path: CefString::from(config.profile_dir.as_str()),
            no_sandbox: 0,
            command_line_args_disabled: 1,
            ..Default::default()
        };

        if initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            sandbox_info,
        ) == 0
        {
            return Err(format!("CEF initialization ended with code {}", get_exit_code()).into());
        }

        run_message_loop();
        shutdown();
        Ok(())
    }
}

pub fn launch_browser_shell(
    config: BrowserShellConfig,
    runtime: HarborRuntime,
    _sandbox_info: *mut u8,
) -> Result<(), Box<dyn std::error::Error>> {
    // Verify embedded UI routes resolve cleanly at startup.
    let _chrome = resolve_harbor_url("harbor://chrome").expect("embedded chrome.html");
    let _home = resolve_harbor_url("harbor://home").expect("embedded internal.html");
    let _autofill_len = AUTOFILL_CONTENT_SCRIPT.len();

    #[cfg(feature = "cef-runtime")]
    {
        tracing::info!(
            profile = %config.profile_dir,
            homepage = %config.homepage,
            chrome_height = CHROME_HEIGHT_PX,
            "starting Harbor CEF runtime"
        );
        runtime_impl::run(config, runtime, _sandbox_info)
    }

    #[cfg(not(feature = "cef-runtime"))]
    {
        let layout = runtime
            .tabs
            .lock()
            .expect("tabs")
            .compute_layout(config.initial_width, config.initial_height);
        let blocked = runtime.adblock.lock().expect("adblock").blocked_total();
        let script_count = runtime.scripts.lock().expect("scripts").len();
        tracing::info!(
            profile = %config.profile_dir,
            homepage = %config.homepage,
            chrome_height = CHROME_HEIGHT_PX,
            content_height = layout.primary_rect.height,
            scripts_loaded = script_count,
            blocked_total = blocked,
            "Harbor core initialized (use build.cmd for the GUI bundle)"
        );
        Ok(())
    }
}

#[cfg(feature = "cef-runtime")]
pub fn early_sub_process_check(sandbox_info: *mut u8) -> Option<i32> {
    runtime_impl::early_sub_process_check(sandbox_info)
}

#[cfg(not(feature = "cef-runtime"))]
pub fn early_sub_process_check(_sandbox_info: *mut u8) -> Option<i32> {
    None
}
