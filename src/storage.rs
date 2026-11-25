use std::path::Path;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use keyring::Entry;
use rusqlite::{Connection, OptionalExtension, Result, params};
use shared_protocol::{
    crypto::{MasterKey, decrypt_envelope, encrypt_envelope, random_bytes, sha256_hex},
    types::{EncryptedEnvelope, ItemType},
};
use uuid::Uuid;
use zeroize::Zeroizing;

const KEYRING_SERVICE: &str = "harbor-browser-local";
const ENCRYPTED_PREFIX: &str = "harbor:v1:";

#[derive(Debug, Clone)]
pub struct LocalRecord {
    pub id: Uuid,
    pub item_type: ItemType,
    pub payload_json: String,
    pub deleted: bool,
    pub dirty: bool,
    pub updated_at: i64,
}

pub struct LocalStore {
    conn: Connection,
    key: MasterKey,
    pub profile_id: String,
}

fn storage_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn load_or_create_db_key(profile_id: &str, has_encrypted_data: bool) -> Result<MasterKey> {
    let entry = Entry::new(KEYRING_SERVICE, profile_id).map_err(storage_error)?;
    match entry.get_password() {
        Ok(existing) => {
            let existing = Zeroizing::new(existing);
            let bytes = Zeroizing::new(B64.decode(existing.as_bytes()).map_err(storage_error)?);
            let key: &[u8; 32] = bytes.as_slice().try_into().map_err(storage_error)?;
            Ok(MasterKey::from_bytes(*key))
        }
        Err(keyring::Error::NoEntry) if !has_encrypted_data => {
            let raw = Zeroizing::new(random_bytes::<32>().map_err(storage_error)?);
            let encoded = Zeroizing::new(B64.encode(raw.as_slice()));
            entry.set_password(&encoded).map_err(storage_error)?;
            Ok(MasterKey::from_bytes(*raw))
        }
        Err(error) => Err(storage_error(error)),
    }
}

impl LocalStore {
    /// Record payloads are authenticated and encrypted with a per-profile credential.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::initialize(&conn)?;
        let canonical = path.canonicalize().map_err(storage_error)?;
        let profile_id = sha256_hex(canonical.to_string_lossy().to_lowercase().as_bytes());
        let encrypted = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_items WHERE payload_json LIKE 'harbor:v1:%')",
            [],
            |row| row.get(0),
        )?;
        let key = load_or_create_db_key(&profile_id, encrypted)?;
        let store = Self {
            conn,
            key,
            profile_id,
        };
        store.encrypt_legacy_records()?;
        Ok(store)
    }

    fn initialize(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA secure_delete = ON;
             CREATE TABLE IF NOT EXISTS local_items (
                 id TEXT PRIMARY KEY,
                 item_type TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 deleted INTEGER NOT NULL DEFAULT 0,
                 dirty INTEGER NOT NULL DEFAULT 1,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sync_state (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS adblock_etag (
                 list_url TEXT PRIMARY KEY,
                 etag TEXT NOT NULL,
                 raw_rules TEXT NOT NULL
             );",
        )
    }

    #[cfg(test)]
    pub(crate) fn test_store(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::initialize(&conn)?;
        Ok(Self {
            conn,
            key: MasterKey::from_bytes([7; 32]),
            profile_id: "test".into(),
        })
    }

    fn encrypt_legacy_records(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, payload_json, deleted, dirty, updated_at
             FROM local_items WHERE payload_json NOT LIKE 'harbor:v1:%'",
        )?;
        let legacy = stmt
            .query_map([], read_record)?
            .collect::<Result<Vec<_>>>()?;
        drop(stmt);
        if legacy.is_empty() {
            return Ok(());
        }
        self.transaction(|store| {
            for record in &legacy {
                store.upsert_lww(
                    record.id,
                    record.item_type,
                    &record.payload_json,
                    record.deleted,
                    record.dirty,
                    record.updated_at,
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })?;
        self.conn.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
        )
    }

    pub fn transaction<T, E: From<rusqlite::Error>>(
        &self,
        operation: impl FnOnce(&Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        let transaction = self.conn.unchecked_transaction()?;
        let value = operation(self)?;
        transaction.commit()?;
        Ok(value)
    }

    fn open_record(&self, mut record: LocalRecord) -> Result<LocalRecord> {
        let encoded = record
            .payload_json
            .strip_prefix(ENCRYPTED_PREFIX)
            .ok_or_else(|| storage_error(std::io::Error::other("unencrypted local record")))?;
        let envelope: EncryptedEnvelope = serde_json::from_str(encoded).map_err(storage_error)?;
        if envelope.id != record.id
            || envelope.item_type != record.item_type
            || envelope.deleted != record.deleted
            || envelope.updated_at != record.updated_at
        {
            return Err(storage_error(std::io::Error::other(
                "local record metadata mismatch",
            )));
        }
        let bytes = decrypt_envelope(&self.key, &envelope).map_err(storage_error)?;
        record.payload_json = String::from_utf8(bytes).map_err(storage_error)?;
        Ok(record)
    }

    /// Last-Write-Wins (LWW) upsert based on `updated_at` timestamp.
    /// Returns true if the record was written or updated.
    pub fn upsert_lww(
        &self,
        id: Uuid,
        item_type: ItemType,
        payload_json: &str,
        deleted: bool,
        dirty: bool,
        updated_at: i64,
    ) -> Result<bool> {
        let envelope = encrypt_envelope(
            &self.key,
            id,
            item_type,
            deleted,
            payload_json.as_bytes(),
            updated_at,
        )
        .map_err(storage_error)?;
        let payload_json = format!(
            "{ENCRYPTED_PREFIX}{}",
            serde_json::to_string(&envelope).map_err(storage_error)?
        );
        let affected = self.conn.execute(
            "INSERT INTO local_items (id, item_type, payload_json, deleted, dirty, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               item_type = excluded.item_type,
               payload_json = excluded.payload_json,
               deleted = excluded.deleted,
               dirty = excluded.dirty,
               updated_at = excluded.updated_at
             WHERE excluded.updated_at >= local_items.updated_at",
            params![
                id.to_string(),
                item_type.as_str(),
                payload_json,
                i64::from(deleted),
                i64::from(dirty),
                updated_at
            ],
        )?;
        Ok(affected > 0)
    }

    pub fn list_by_type(&self, item_type: ItemType) -> Result<Vec<LocalRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, payload_json, deleted, dirty, updated_at
             FROM local_items
             WHERE item_type = ?1 AND deleted = 0
             ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(params![item_type.as_str()], read_record)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(self.open_record(r?)?);
        }
        Ok(out)
    }

    pub fn count_by_type(&self, item_type: ItemType) -> Result<u64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM local_items WHERE item_type = ?1 AND deleted = 0",
            [item_type.as_str()],
            |row| row.get(0),
        )
    }

    pub fn record(&self, id: Uuid, item_type: ItemType) -> Result<Option<LocalRecord>> {
        self.conn
            .query_row(
                "SELECT id, item_type, payload_json, deleted, dirty, updated_at
             FROM local_items WHERE id = ?1 AND item_type = ?2 AND deleted = 0",
                params![id.to_string(), item_type.as_str()],
                read_record,
            )
            .optional()?
            .map(|record| self.open_record(record))
            .transpose()
    }

    pub fn save_record(
        &self,
        id: Uuid,
        item_type: ItemType,
        payload: &str,
        deleted: bool,
        now: i64,
    ) -> Result<()> {
        let previous: Option<i64> = self
            .conn
            .query_row(
                "SELECT updated_at FROM local_items WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let timestamp = previous.map_or(now, |previous| now.max(previous.saturating_add(1)));
        self.upsert_lww(id, item_type, payload, deleted, true, timestamp)?;
        Ok(())
    }

    pub fn filter_cache(&self, url: &str) -> Result<Option<String>> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT raw_rules FROM adblock_etag WHERE list_url = ?1",
                [url],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let Some(raw) = raw.strip_prefix(ENCRYPTED_PREFIX) else {
            return Ok(None);
        };
        let envelope: EncryptedEnvelope = serde_json::from_str(raw).map_err(storage_error)?;
        if envelope.id != filter_cache_id(url) {
            return Ok(None);
        }
        let rules = decrypt_envelope(&self.key, &envelope).map_err(storage_error)?;
        Ok(Some(String::from_utf8(rules).map_err(storage_error)?))
    }

    pub fn save_filter_cache(&self, url: &str, rules: &str) -> Result<()> {
        let envelope = encrypt_envelope(
            &self.key,
            filter_cache_id(url),
            ItemType::AdblockConfig,
            false,
            rules.as_bytes(),
            0,
        )
        .map_err(storage_error)?;
        let encoded = format!(
            "{ENCRYPTED_PREFIX}{}",
            serde_json::to_string(&envelope).map_err(storage_error)?
        );
        self.conn.execute(
            "INSERT INTO adblock_etag (list_url,etag,raw_rules) VALUES (?1,'',?2)
             ON CONFLICT(list_url) DO UPDATE SET raw_rules=excluded.raw_rules",
            params![url, encoded],
        )?;
        Ok(())
    }

    pub fn fetch_dirty(&self) -> Result<Vec<LocalRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_type, payload_json, deleted, dirty, updated_at
             FROM local_items WHERE dirty = 1",
        )?;
        let rows = stmt.query_map([], read_record)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(self.open_record(r?)?);
        }
        Ok(out)
    }

    pub fn mark_clean(&self, records: &[LocalRecord]) -> Result<()> {
        for record in records {
            self.conn.execute(
                "UPDATE local_items SET dirty = 0 WHERE id = ?1 AND updated_at = ?2",
                params![record.id.to_string(), record.updated_at],
            )?;
        }
        Ok(())
    }

    pub fn get_state(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM sync_state WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_state(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_last_revision(&self) -> Result<i64> {
        Ok(self
            .get_state("last_revision")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_last_revision(&self, rev: i64) -> Result<()> {
        self.set_state("last_revision", &rev.to_string())
    }
}

fn filter_cache_id(url: &str) -> Uuid {
    let digest = shared_protocol::crypto::hash_token(url);
    Uuid::from_bytes(digest[..16].try_into().expect("SHA-256 prefix"))
}

fn read_record(row: &rusqlite::Row<'_>) -> Result<LocalRecord> {
    let id: String = row.get(0)?;
    let item_type: String = row.get(1)?;
    Ok(LocalRecord {
        id: Uuid::parse_str(&id).map_err(storage_error)?,
        item_type: ItemType::from_str(&item_type).unwrap_or(ItemType::Setting),
        payload_json: row.get(2)?,
        deleted: row.get::<_, i64>(3)? != 0,
        dirty: row.get::<_, i64>(4)? != 0,
        updated_at: row.get(5)?,
    })
}
