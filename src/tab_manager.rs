use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const CHROME_HEIGHT_PX: i32 = 48;
pub const NO_TAB: u64 = 0;
pub const EMPTY_WORKSPACE: &str = "harbor://empty/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpaceKind {
    Work,
    Personal,
}

impl SpaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Work => "Work",
            Self::Personal => "Personal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Computed layout rectangles for the frameless Harbor window.
/// Popovers use a separate overlay above these fixed browser regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowLayout {
    pub chrome_rect: Rect,
    pub primary_rect: Rect,
    pub secondary_rect: Option<Rect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabEntry {
    pub id: u64,
    pub space: SpaceKind,
    pub title: String,
    pub url: String,
    pub pinned: bool,
    pub loading_progress: u8,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub blocked_on_page: u64,
    #[serde(default)]
    pub audible: bool,
    #[serde(default)]
    pub sleeping: bool,
}

pub struct TabManager {
    next_id: u64,
    pub active_space: SpaceKind,
    pub active_tab_id: u64,
    pub split_tab_id: Option<u64>,
    focused_tab_id: Option<u64>,
    tabs: Vec<TabEntry>,
    /// Maps Harbor tab IDs to CEF browser identifiers once created.
    cef_browser_ids: HashMap<u64, i32>,
    /// Last time each tab was visible (used for background LRU & sleep timers).
    last_active_at: HashMap<u64, Instant>,
}

impl TabManager {
    pub fn new(homepage: &str) -> Self {
        let initial = TabEntry {
            id: 1,
            space: SpaceKind::Personal,
            title: "Home".to_string(),
            url: homepage.to_string(),
            pinned: false,
            loading_progress: 100,
            can_go_back: false,
            can_go_forward: false,
            blocked_on_page: 0,
            audible: false,
            sleeping: false,
        };
        let mut last_active_at = HashMap::new();
        last_active_at.insert(1, Instant::now());
        Self {
            next_id: 2,
            active_space: SpaceKind::Personal,
            active_tab_id: 1,
            split_tab_id: None,
            focused_tab_id: None,
            tabs: vec![initial],
            cef_browser_ids: HashMap::new(),
            last_active_at,
        }
    }

    fn touch_tab(&mut self, id: u64, now: Instant) {
        if id != NO_TAB {
            self.last_active_at.insert(id, now);
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                tab.sleeping = false;
            }
        }
    }

    pub fn open_tab(&mut self, url: impl Into<String>, title: impl Into<String>) -> TabEntry {
        let now = Instant::now();
        let prev = self.active_tab_id;
        self.touch_tab(prev, now);
        let id = self.next_id;
        self.next_id += 1;
        let entry = TabEntry {
            id,
            space: self.active_space,
            title: title.into(),
            url: url.into(),
            pinned: false,
            loading_progress: 0,
            can_go_back: false,
            can_go_forward: false,
            blocked_on_page: 0,
            audible: false,
            sleeping: false,
        };
        self.tabs.push(entry.clone());
        self.active_tab_id = id;
        self.focused_tab_id = None;
        self.last_active_at.insert(id, now);
        entry
    }

    pub fn select_tab(&mut self, id: u64) -> Option<&TabEntry> {
        let now = Instant::now();
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            let tab_space = self.tabs[idx].space;
            if self.split_tab_id == Some(id) {
                self.focused_tab_id = Some(id);
                self.touch_tab(id, now);
                return self.tabs.iter().find(|t| t.id == id);
            }
            self.touch_tab(self.active_tab_id, now);
            if self.active_space != tab_space {
                if let Some(split_id) = self.split_tab_id.take() {
                    self.touch_tab(split_id, now);
                }
            }
            self.active_space = tab_space;
            self.active_tab_id = id;
            self.focused_tab_id = None;
            self.touch_tab(id, now);
            return self.tabs.iter().find(|t| t.id == id);
        }
        None
    }

    pub fn switch_space(&mut self, space: SpaceKind) -> Option<&TabEntry> {
        let now = Instant::now();
        self.touch_tab(self.active_tab_id, now);
        if self.active_space != space {
            if let Some(split_id) = self.split_tab_id.take() {
                self.touch_tab(split_id, now);
            }
        }
        self.focused_tab_id = None;
        self.active_space = space;
        let active_in_space = self
            .tabs
            .iter()
            .any(|t| t.id == self.active_tab_id && t.space == space);
        if !active_in_space {
            if let Some(first) = self.tabs.iter().find(|t| t.space == space) {
                self.active_tab_id = first.id;
            } else {
                self.active_tab_id = NO_TAB;
            }
        }
        self.touch_tab(self.active_tab_id, now);
        self.active_tab()
    }

    pub fn close_tab(&mut self, id: u64) -> bool {
        let Some(pos) = self.tabs.iter().position(|t| t.id == id) else {
            return false;
        };
        self.tabs.remove(pos);
        self.cef_browser_ids.remove(&id);
        self.last_active_at.remove(&id);

        if self.split_tab_id == Some(id) {
            self.split_tab_id = None;
            self.focused_tab_id = None;
        }

        if self.active_tab_id == id {
            let next = self
                .tabs
                .iter()
                .find(|t| t.space == self.active_space && Some(t.id) != self.split_tab_id)
                .map(|t| t.id);
            if let Some(next) = next {
                self.select_tab(next);
            } else if let Some(right) = self.split_tab_id.take() {
                self.select_tab(right);
            } else {
                self.active_tab_id = NO_TAB;
                self.focused_tab_id = None;
            }
        }

        true
    }

    pub fn toggle_split_view(&mut self) {
        if self.split_tab_id.is_some() {
            self.split_tab_id = None;
            self.focused_tab_id = None;
            return;
        }
        if self.active_tab_id == NO_TAB {
            self.open_tab("harbor://home", "Home");
        }
        let other = self
            .tabs
            .iter()
            .find(|t| t.id != self.active_tab_id && t.space == self.active_space)
            .map(|t| t.id);
        self.split_tab_id = other.or_else(|| {
            let primary = self.active_tab_id;
            let right = self.open_tab("harbor://home", "Home").id;
            self.active_tab_id = primary;
            Some(right)
        });
    }

    pub fn focused_tab_id(&self) -> u64 {
        self.focused_tab_id.unwrap_or(self.active_tab_id)
    }

    pub fn focus_cef_browser(&mut self, cef_id: i32) {
        if self.cef_browser_ids.get(&self.active_tab_id) == Some(&cef_id) {
            self.focused_tab_id = None;
        } else if let Some(right) = self.split_tab_id {
            if self.cef_browser_ids.get(&right) == Some(&cef_id) {
                self.focused_tab_id = Some(right);
            }
        }
    }

    pub fn split_tab(&self) -> Option<&TabEntry> {
        self.tab(self.split_tab_id?)
    }

    pub fn tab(&self, id: u64) -> Option<&TabEntry> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn unbind_cef_browser(&mut self, cef_id: i32) {
        self.cef_browser_ids.retain(|_, id| *id != cef_id);
    }

    pub fn bind_cef_browser(&mut self, tab_id: u64, cef_id: i32) {
        self.cef_browser_ids.retain(|_, id| *id != cef_id);
        if self.tab(tab_id).is_some() {
            self.cef_browser_ids.insert(tab_id, cef_id);
        }
    }

    pub fn tab_id_for_cef(&self, cef_id: i32) -> Option<u64> {
        self.cef_browser_ids
            .iter()
            .find_map(|(&tab_id, &cid)| (cid == cef_id).then_some(tab_id))
    }

    pub fn update_tab_navigation(
        &mut self,
        tab_id: u64,
        url: Option<String>,
        title: Option<String>,
        progress: Option<u8>,
    ) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            if let Some(u) = url {
                if tab.url != u || progress == Some(15) {
                    tab.blocked_on_page = 0;
                }
                tab.url = u;
            }
            if let Some(t) = title {
                tab.title = t;
            }
            if let Some(p) = progress {
                tab.loading_progress = p;
            }
        }
    }

    pub fn update_tab_history(&mut self, tab_id: u64, can_go_back: bool, can_go_forward: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.can_go_back = can_go_back;
            tab.can_go_forward = can_go_forward;
        }
    }

    pub fn record_blocked_request(&mut self, tab_id: u64) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.blocked_on_page += 1;
        }
    }

    pub fn set_tab_audible(&mut self, cef_id: i32, audible: bool) -> bool {
        let Some(tab_id) = self.tab_id_for_cef(cef_id) else {
            return false;
        };
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            if tab.audible != audible {
                tab.audible = audible;
                return true;
            }
        }
        false
    }

    pub fn set_tab_sleeping(&mut self, tab_id: u64, sleeping: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.sleeping = sleeping;
            if sleeping {
                tab.audible = false;
            }
        }
    }

    /// Returns live background tab IDs that should be put to sleep based on settings.
    pub fn tabs_to_sleep(
        &self,
        live_tab_ids: &[u64],
        now: Instant,
        background_enabled: bool,
        max_background: u32,
        sleep_after_mins: u32,
        preserve_audio: bool,
    ) -> Vec<u64> {
        let mut to_sleep = Vec::new();
        let mut candidates: Vec<(u64, Instant)> = Vec::new();

        for &id in live_tab_ids {
            if id == self.active_tab_id || self.split_tab_id == Some(id) {
                continue;
            }
            let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
                to_sleep.push(id);
                continue;
            };
            if !background_enabled {
                to_sleep.push(id);
                continue;
            }
            if tab.pinned || tab.url.starts_with("harbor://") || (preserve_audio && tab.audible) {
                continue;
            }
            let last_seen = self.last_active_at.get(&id).copied().unwrap_or(now);
            if sleep_after_mins > 0
                && now.saturating_duration_since(last_seen)
                    >= Duration::from_secs(u64::from(sleep_after_mins) * 60)
            {
                to_sleep.push(id);
            } else {
                candidates.push((id, last_seen));
            }
        }

        if background_enabled && max_background > 0 {
            let limit = max_background as usize;
            if candidates.len() > limit {
                candidates.sort_by_key(|&(_, ts)| ts);
                let excess = candidates.len() - limit;
                for (id, _) in candidates.into_iter().take(excess) {
                    to_sleep.push(id);
                }
            }
        }

        to_sleep
    }

    pub fn active_tab(&self) -> Option<&TabEntry> {
        self.tabs.iter().find(|t| t.id == self.active_tab_id)
    }

    pub fn space_tabs(&self, space: SpaceKind) -> Vec<&TabEntry> {
        self.tabs.iter().filter(|t| t.space == space).collect()
    }

    /// Computes pixel-exact rectangles for the frameless window.
    pub fn compute_layout(&self, window_width: i32, window_height: i32) -> WindowLayout {
        let w = window_width.max(320);
        let h = window_height.max(240);
        let content_y = CHROME_HEIGHT_PX;
        let content_h = (h - content_y).max(0);

        let chrome_rect = Rect {
            x: 0,
            y: 0,
            width: w,
            height: CHROME_HEIGHT_PX,
        };

        if self.split_tab_id.is_some() {
            let left_w = w / 2;
            let right_w = w - left_w;
            WindowLayout {
                chrome_rect,
                primary_rect: Rect {
                    x: 0,
                    y: content_y,
                    width: left_w,
                    height: content_h,
                },
                secondary_rect: Some(Rect {
                    x: left_w,
                    y: content_y,
                    width: right_w,
                    height: content_h,
                }),
            }
        } else {
            WindowLayout {
                chrome_rect,
                primary_rect: Rect {
                    x: 0,
                    y: content_y,
                    width: w,
                    height: content_h,
                },
                secondary_rect: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NO_TAB, SpaceKind, TabManager};

    #[test]
    fn split_opens_a_second_tab_and_follows_pane_focus() {
        let mut tabs = TabManager::new("harbor://home");
        let left = tabs.active_tab_id;
        tabs.bind_cef_browser(left, 10);
        tabs.toggle_split_view();
        let right = tabs.split_tab_id.unwrap();
        assert_ne!(left, right);
        assert_eq!(tabs.focused_tab_id(), left);
        tabs.bind_cef_browser(right, 20);
        tabs.focus_cef_browser(20);
        assert_eq!(tabs.focused_tab_id(), right);
        tabs.close_tab(left);
        assert!(tabs.split_tab_id.is_none());
        assert_eq!(tabs.focused_tab_id(), right);
    }

    #[test]
    fn closing_the_last_tab_keeps_the_workspace_empty() {
        let mut tabs = TabManager::new("harbor://home");
        let personal = tabs.active_tab_id;
        let second_personal = tabs.open_tab("https://example.com", "Example").id;
        assert!(tabs.close_tab(second_personal));
        assert!(tabs.tab(second_personal).is_none());
        assert!(!tabs.close_tab(second_personal));
        assert!(tabs.switch_space(SpaceKind::Work).is_none());
        let work = tabs.open_tab("harbor://home", "Home").id;
        assert!(tabs.switch_space(SpaceKind::Personal).is_some());
        assert!(tabs.close_tab(personal));
        assert_eq!(tabs.active_tab_id, NO_TAB);
        assert_eq!(tabs.active_space, SpaceKind::Personal);
        assert_eq!(
            tabs.switch_space(SpaceKind::Work).map(|tab| tab.id),
            Some(work)
        );
        assert!(tabs.close_tab(work));
        assert!(tabs.space_tabs(SpaceKind::Work).is_empty());
        assert!(tabs.switch_space(SpaceKind::Personal).is_none());
    }

    #[test]
    fn background_tabs_respect_timeout_lru_limit_and_audio() {
        use std::time::{Duration, Instant};

        let mut tabs = TabManager::new("https://example.com/1");
        let t1 = tabs.active_tab_id;
        let t2 = tabs.open_tab("https://open.spotify.com", "Spotify").id;
        tabs.bind_cef_browser(t2, 200);
        assert!(tabs.set_tab_audible(200, true));
        let t3 = tabs.open_tab("https://example.com/3", "Three").id;
        let t4 = tabs.open_tab("https://example.com/4", "Four").id;
        let t5 = tabs.open_tab("https://example.com/5", "Five").id;

        let live = vec![t1, t2, t3, t4, t5];
        let now = Instant::now();

        // With limit=2, t2 is audible so excluded; t1 and t3 are oldest -> t1 evicted first.
        let sleep_lru = tabs.tabs_to_sleep(&live, now, true, 1, 30, true);
        assert_eq!(sleep_lru.len(), 2);
        assert!(sleep_lru.contains(&t1));
        assert!(sleep_lru.contains(&t3));
        assert!(!sleep_lru.contains(&t2));
        assert!(!sleep_lru.contains(&t5));

        // After 31 minutes, t1, t3, t4 time out, while t2 (audible) and t5 (active) stay awake.
        let later = now + Duration::from_secs(31 * 60);
        let sleep_timeout = tabs.tabs_to_sleep(&live, later, true, 0, 30, true);
        assert_eq!(sleep_timeout, vec![t1, t3, t4]);

        // When background tabs are disabled, all non-visible tabs sleep immediately.
        let sleep_disabled = tabs.tabs_to_sleep(&live, now, false, 8, 30, true);
        assert_eq!(sleep_disabled, vec![t1, t2, t3, t4]);
    }
}
