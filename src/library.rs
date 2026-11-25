use serde_json::{Value, json};
use shared_protocol::types::{BookmarkPayload, HistoryPayload, ItemType, SettingPayload};
use uuid::Uuid;

use crate::{ipc_router::now_unix, storage::LocalStore};

pub type LibraryResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const FOLDER_KEY: &str = "harbor:bookmark-folder";

pub fn bookmarks(store: &LocalStore) -> LibraryResult<Value> {
    let folders: Vec<_> = store
        .list_by_type(ItemType::Setting)?
        .into_iter()
        .filter_map(|record| {
            let data: SettingPayload = serde_json::from_str(&record.payload_json).ok()?;
            (data.key == FOLDER_KEY).then(|| json!({ "id": record.id, "name": data.value }))
        })
        .collect();
    let bookmarks: Vec<_> = store.list_by_type(ItemType::Bookmark)?.into_iter()
        .filter_map(|record| {
            let data: BookmarkPayload = serde_json::from_str(&record.payload_json).ok()?;
            Some(json!({ "id": record.id, "title": data.title, "url": data.url, "folder": data.folder }))
        }).collect();
    Ok(json!({ "bookmarks": bookmarks, "folders": folders }))
}

pub fn history(store: &LocalStore) -> LibraryResult<Value> {
    let history: Vec<_> = store.list_by_type(ItemType::History)?.into_iter()
        .filter_map(|record| {
            let data: HistoryPayload = serde_json::from_str(&record.payload_json).ok()?;
            Some(json!({ "id": record.id, "title": data.title, "url": data.url, "visited_at": data.visited_at }))
        }).collect();
    Ok(json!(history))
}

fn check_folder(store: &LocalStore, id: Uuid) -> LibraryResult<()> {
    let record = store
        .record(id, ItemType::Setting)?
        .ok_or("This folder no longer exists.")?;
    let folder: SettingPayload = serde_json::from_str(&record.payload_json)?;
    if folder.key != FOLDER_KEY {
        return Err("This folder no longer exists.".into());
    }
    Ok(())
}

pub fn save_folder(store: &LocalStore, id: Option<Uuid>, name: &str) -> LibraryResult<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err("Enter a folder name between 1 and 120 characters.".into());
    }
    if let Some(id) = id {
        check_folder(store, id)?;
    }
    let data = SettingPayload {
        key: FOLDER_KEY.into(),
        value: name.into(),
    };
    store.save_record(
        id.unwrap_or_else(Uuid::new_v4),
        ItemType::Setting,
        &serde_json::to_string(&data)?,
        false,
        now_unix(),
    )?;
    Ok(())
}

pub fn delete_folder(store: &LocalStore, id: Uuid) -> LibraryResult<()> {
    check_folder(store, id)?;
    let folder = id.to_string();
    store.transaction(|store| -> LibraryResult<()> {
        for record in store.list_by_type(ItemType::Bookmark)? {
            let mut bookmark: BookmarkPayload = serde_json::from_str(&record.payload_json)?;
            if bookmark.folder.as_deref() == Some(folder.as_str()) {
                bookmark.folder = None;
                store.save_record(
                    record.id,
                    ItemType::Bookmark,
                    &serde_json::to_string(&bookmark)?,
                    false,
                    now_unix(),
                )?;
            }
        }
        store.save_record(id, ItemType::Setting, "{}", true, now_unix())?;
        Ok(())
    })
}

pub fn save_bookmark(
    store: &LocalStore,
    id: Option<Uuid>,
    title: &str,
    url: &str,
    folder: Option<Uuid>,
) -> LibraryResult<()> {
    if let Some(id) = id {
        store
            .record(id, ItemType::Bookmark)?
            .ok_or("This bookmark no longer exists.")?;
    }
    if let Some(folder) = folder {
        check_folder(store, folder)?;
    }
    let url = crate::workspace::home_address(url)
        .ok_or("Enter a valid web or Harbor workspace address.")?;
    let title = title.trim();
    let bookmark = BookmarkPayload {
        title: if title.is_empty() {
            url.clone()
        } else {
            title.into()
        },
        url,
        folder: folder.map(|id| id.to_string()),
    };
    store.save_record(
        id.unwrap_or_else(Uuid::new_v4),
        ItemType::Bookmark,
        &serde_json::to_string(&bookmark)?,
        false,
        now_unix(),
    )?;
    Ok(())
}

pub fn move_bookmark(store: &LocalStore, id: Uuid, folder: Option<Uuid>) -> LibraryResult<()> {
    if let Some(folder) = folder {
        check_folder(store, folder)?;
    }
    let record = store
        .record(id, ItemType::Bookmark)?
        .ok_or("This bookmark no longer exists.")?;
    let mut bookmark: BookmarkPayload = serde_json::from_str(&record.payload_json)?;
    bookmark.folder = folder.map(|id| id.to_string());
    store.save_record(
        id,
        ItemType::Bookmark,
        &serde_json::to_string(&bookmark)?,
        false,
        now_unix(),
    )?;
    Ok(())
}

pub fn delete_record(store: &LocalStore, id: Uuid, item_type: ItemType) -> LibraryResult<()> {
    if store.record(id, item_type)?.is_some() {
        store.save_record(id, item_type, "{}", true, now_unix())?;
    }
    Ok(())
}

pub fn clear_history(
    store: &LocalStore,
    from: Option<i64>,
    until: Option<i64>,
) -> LibraryResult<usize> {
    if from.zip(until).is_some_and(|(from, until)| from >= until) {
        return Err("The start date must be on or before the end date.".into());
    }
    store.transaction(|store| -> LibraryResult<usize> {
        let mut removed = 0;
        for record in store.list_by_type(ItemType::History)? {
            let item: HistoryPayload = serde_json::from_str(&record.payload_json)?;
            if from.is_none_or(|date| item.visited_at >= date)
                && until.is_none_or(|date| item.visited_at < date)
            {
                store.save_record(record.id, ItemType::History, "{}", true, now_unix())?;
                removed += 1;
            }
        }
        Ok(removed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_protocol::types::AdblockConfigPayload;
    use std::path::Path;

    #[test]
    fn folders_move_bookmarks_and_keep_them_when_deleted() {
        let store = LocalStore::test_store(Path::new(":memory:")).unwrap();
        save_folder(&store, None, "Work").unwrap();
        let folder = Uuid::parse_str(
            bookmarks(&store).unwrap()["folders"][0]["id"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        save_bookmark(&store, None, "Example", "example.com", None).unwrap();
        let bookmark = Uuid::parse_str(
            bookmarks(&store).unwrap()["bookmarks"][0]["id"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        move_bookmark(&store, bookmark, Some(folder)).unwrap();
        save_folder(&store, Some(folder), "Renamed").unwrap();
        assert_eq!(
            bookmarks(&store).unwrap()["bookmarks"][0]["folder"],
            folder.to_string()
        );
        assert_eq!(bookmarks(&store).unwrap()["folders"][0]["name"], "Renamed");
        delete_folder(&store, folder).unwrap();
        let result = bookmarks(&store).unwrap();
        assert_eq!(result["bookmarks"][0]["folder"], Value::Null);
        assert!(result["folders"].as_array().unwrap().is_empty());
        assert_eq!(store.count_by_type(ItemType::Bookmark).unwrap(), 1);
    }

    #[test]
    fn deletes_history_by_date_and_keeps_filter_settings_across_reopen() {
        let database =
            std::env::temp_dir().join(format!("harbor-workspace-test-{}.db", Uuid::new_v4()));
        {
            let store = LocalStore::test_store(&database).unwrap();
            for time in [10, 20, 30] {
                let visit = HistoryPayload {
                    title: "Visit".into(),
                    url: "https://example.com".into(),
                    visited_at: time,
                };
                store
                    .save_record(
                        Uuid::new_v4(),
                        ItemType::History,
                        &serde_json::to_string(&visit).unwrap(),
                        false,
                        time,
                    )
                    .unwrap();
            }
            assert_eq!(clear_history(&store, Some(20), Some(30)).unwrap(), 1);
            let config = AdblockConfigPayload {
                list_urls: vec!["https://example.com/filters".into()],
                custom_rules: vec!["||example.net^".into()],
                allowlisted_domains: vec![],
            };
            crate::workspace::store_adblock(&store, &config).unwrap();
        }
        {
            let store = LocalStore::test_store(&database).unwrap();
            assert_eq!(store.count_by_type(ItemType::History).unwrap(), 2);
            let (lists, custom, _) = crate::load_saved_adblock_config(&store);
            assert_eq!(lists, ["https://example.com/filters"]);
            assert_eq!(custom, ["||example.net^"]);
            assert_eq!(clear_history(&store, None, None).unwrap(), 2);
        }
        std::fs::remove_file(database).unwrap();
    }
}
