use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use ed25519_dalek::SigningKey;
use keyring::Entry;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared_protocol::{
    crypto::{
        MasterKey, decrypt_envelope, encrypt_envelope, generate_device_keypair, random_bytes,
        sign_handshake,
    },
    types::*,
};
use thiserror::Error;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::storage::LocalStore;
use crate::tls_pin::build_pinned_tls13_config;

const KEYRING_SERVICE: &str = "harbor-browser-sync";

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("tls configuration error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("crypto error: {0}")]
    Crypto(#[from] shared_protocol::crypto::CryptoError),
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("storage error: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("invalid stored credential or server payload: {0}")]
    InvalidCredential(&'static str),
    #[error("device has been revoked by sync server; re-pairing required")]
    DeviceRevoked,
    #[error("server rejected request with status {0}")]
    Rejected(u16),
    #[error("{0}")]
    Address(&'static str),
}

pub struct SyncClient {
    server_url: String,
    http: Client,
    profile_id: String,
}

struct SyncBatch {
    scripts_changed: bool,
    filters_changed: bool,
    more_to_upload: bool,
}

impl SyncClient {
    /// Builds a strict HTTPS-only sync client. When `pinned_cert_sha256` is provided,
    /// TLS 1.3 certificate fingerprint pinning is enforced on every request.
    pub fn new(
        server_url: impl Into<String>,
        pinned_cert_sha256: Option<String>,
        profile_id: String,
    ) -> Result<Self, SyncError> {
        let server_url = normalize_server_address(&server_url.into())?;
        let mut builder = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .min_tls_version(reqwest::tls::Version::TLS_1_3)
            .timeout(Duration::from_secs(20));
        if let Some(pin) = pinned_cert_sha256 {
            if pin.len() != 64 || !pin.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(SyncError::Address(
                    "Enter the 64-character SHA-256 fingerprint of your server certificate.",
                ));
            }
            let tls_cfg = build_pinned_tls13_config(pin)?;
            builder = builder.use_preconfigured_tls(tls_cfg);
        } else {
            builder = builder.use_rustls_tls();
        }
        let http = builder.build()?;
        Ok(Self {
            server_url,
            http,
            profile_id,
        })
    }

    async fn check_server(&self) -> Result<(), SyncError> {
        let mut response = self.http.get(format!("{}/health", self.server_url))
            .send().await.map_err(|error| {
                if error.is_timeout() {
                    SyncError::Address("The server did not respond. Check that it is running and reachable from this device.")
                } else {
                    SyncError::Address("Harbor could not connect securely. Check the address, network, and server certificate.")
                }
            })?;
        let unexpected_response = || {
            SyncError::Address("This address did not return a Harbor sync server health response.")
        };
        if !response.status().is_success() {
            return Err(unexpected_response());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > 16 {
                return Err(unexpected_response());
            }
            body.extend_from_slice(&chunk);
        }
        if std::str::from_utf8(&body).is_ok_and(|text| text.trim() == "ok") {
            Ok(())
        } else {
            Err(unexpected_response())
        }
    }

    /// Returns true if this device already has a paired identity in the OS credential vault.
    fn credential(&self, key: &str) -> Result<Entry, keyring::Error> {
        Entry::new(KEYRING_SERVICE, &format!("{}:{key}", self.profile_id))
    }

    pub fn is_paired(&self) -> bool {
        self.credential("identity")
            .and_then(|entry| entry.get_password())
            .is_ok()
    }

    /// Clears local sync credentials after a device revocation so the user can re-pair cleanly.
    pub fn clear_credentials(&self) {
        for key in ["identity", "refresh_token"] {
            if let Ok(entry) = self.credential(key) {
                let _ = entry.delete_credential();
            }
        }
    }

    fn check_status(status: reqwest::StatusCode) -> Result<(), SyncError> {
        if status.is_success() {
            return Ok(());
        }
        if status.as_u16() == 403 {
            return Err(SyncError::DeviceRevoked);
        }
        Err(SyncError::Rejected(status.as_u16()))
    }

    /// Pairs this browser with the self-hosted server, stores the Ed25519 private key
    /// and initial rotating refresh token in the OS vault, and returns any existing KDF salt.
    pub async fn pair_device(
        &self,
        pairing_code: &str,
        device_name: &str,
    ) -> Result<(Uuid, Option<VaultMeta>), SyncError> {
        let signing_key = generate_device_keypair()?;
        let public_key_b64 = B64.encode(signing_key.verifying_key().to_bytes());

        let resp = self
            .http
            .post(format!("{}/pair", self.server_url))
            .json(&PairRequest {
                pairing_code: pairing_code.to_string(),
                device_name: device_name.to_string(),
                public_key_b64,
            })
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SyncError::Address(
                "The pairing code is invalid, expired, or already used. Generate a new code on the server.",
            ));
        }
        Self::check_status(resp.status())?;
        let paired: PairResponse = resp.json().await?;

        let priv_b64 = Zeroizing::new(B64.encode(signing_key.to_bytes()));
        let token = Zeroizing::new(paired.refresh_token);

        self.credential("refresh_token")?.set_password(&token)?;
        let identity = Zeroizing::new(format!("{}:{}", paired.device_id, priv_b64.as_str()));
        self.credential("identity")?.set_password(&identity)?;

        Ok((paired.device_id, paired.vault_meta))
    }

    async fn perform_handshake(&self) -> Result<Zeroizing<String>, SyncError> {
        let identity = Zeroizing::new(self.credential("identity")?.get_password()?);
        let (device_id_str, priv_b64) = identity
            .split_once(':')
            .ok_or(SyncError::InvalidCredential("malformed device identity"))?;
        let device_id = Uuid::parse_str(device_id_str)
            .map_err(|_| SyncError::InvalidCredential("malformed device_id"))?;
        let refresh_token = Zeroizing::new(self.credential("refresh_token")?.get_password()?);

        let priv_vec = Zeroizing::new(
            B64.decode(priv_b64)
                .map_err(|_| SyncError::InvalidCredential("invalid base64 private key"))?,
        );
        if priv_vec.len() != 32 {
            return Err(SyncError::InvalidCredential("private key length != 32"));
        }
        let mut priv_bytes = Zeroizing::new([0u8; 32]);
        priv_bytes.copy_from_slice(&priv_vec);
        let signing_key = SigningKey::from_bytes(&priv_bytes);

        let ch_resp = self
            .http
            .post(format!("{}/auth/challenge", self.server_url))
            .json(&ChallengeRequest { device_id })
            .send()
            .await?;
        Self::check_status(ch_resp.status())?;
        let challenge: ChallengeResponse = ch_resp.json().await?;
        let nonce_raw = B64
            .decode(&challenge.server_nonce_b64)
            .map_err(|_| SyncError::InvalidCredential("invalid server nonce"))?;

        let signature_b64 = sign_handshake(&signing_key, device_id, &nonce_raw, &refresh_token);
        let hs_resp = self
            .http
            .post(format!("{}/auth/handshake", self.server_url))
            .json(&HandshakeRequest {
                device_id,
                server_nonce_b64: challenge.server_nonce_b64,
                refresh_token: refresh_token.to_string(),
                signature_b64,
            })
            .send()
            .await?;
        Self::check_status(hs_resp.status())?;
        let handshake: HandshakeResponse = hs_resp.json().await?;

        let next_token = Zeroizing::new(handshake.next_refresh_token);
        self.credential("refresh_token")?
            .set_password(&next_token)?;

        Ok(Zeroizing::new(handshake.access_token))
    }

    /// Initializes the shared 16-byte Argon2id salt on the server if this is the first device.
    pub async fn ensure_vault_salt(&self) -> Result<[u8; 16], SyncError> {
        let access_token = self.perform_handshake().await?;
        if let Some(salt) = self.read_vault_salt(&access_token).await? {
            return Ok(salt);
        }
        let new_salt = random_bytes::<16>()?;
        let meta = VaultMeta {
            kdf_salt_b64: B64.encode(new_salt),
            kdf_version: 1,
        };
        let put_resp = self
            .http
            .put(format!("{}/vault/meta", self.server_url))
            .bearer_auth(access_token.as_str())
            .json(&meta)
            .send()
            .await?;
        if put_resp.status() == reqwest::StatusCode::CONFLICT {
            return self
                .read_vault_salt(&access_token)
                .await?
                .ok_or(SyncError::InvalidCredential(
                    "server did not return the shared vault salt",
                ));
        }
        Self::check_status(put_resp.status())?;
        Ok(new_salt)
    }

    async fn read_vault_salt(&self, access_token: &str) -> Result<Option<[u8; 16]>, SyncError> {
        let resp = self
            .http
            .get(format!("{}/vault/meta", self.server_url))
            .bearer_auth(access_token)
            .send()
            .await?;
        Self::check_status(resp.status())?;
        if let Some(meta) = resp.json::<Option<VaultMeta>>().await? {
            if meta.kdf_version != 1 {
                return Err(SyncError::InvalidCredential(
                    "unsupported vault key derivation version",
                ));
            }
            let bytes: [u8; 16] = B64
                .decode(&meta.kdf_salt_b64)
                .map_err(|_| SyncError::InvalidCredential("invalid base64 kdf salt"))?
                .try_into()
                .map_err(|_| SyncError::InvalidCredential("kdf salt length != 16"))?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    pub async fn list_devices(&self) -> Result<Vec<DeviceSummary>, SyncError> {
        let access_token = self.perform_handshake().await?;
        let resp = self
            .http
            .get(format!("{}/devices", self.server_url))
            .bearer_auth(access_token.as_str())
            .send()
            .await?;
        Self::check_status(resp.status())?;
        Ok(resp.json().await?)
    }

    pub async fn revoke_device(&self, target_device_id: Uuid) -> Result<(), SyncError> {
        let access_token = self.perform_handshake().await?;
        let resp = self
            .http
            .post(format!("{}/devices/revoke", self.server_url))
            .bearer_auth(access_token.as_str())
            .json(&RevokeDeviceRequest {
                device_id: target_device_id,
            })
            .send()
            .await?;
        Self::check_status(resp.status())?;
        Ok(())
    }

    async fn exchange(
        &self,
        access_token: &str,
        request: &SyncRequest,
    ) -> Result<SyncResponse, SyncError> {
        let response = self
            .http
            .post(format!("{}/sync", self.server_url))
            .bearer_auth(access_token)
            .json(request)
            .send()
            .await?;
        Self::check_status(response.status())?;
        Ok(response.json().await?)
    }

    async fn verify_vault_key(&self, key: &MasterKey) -> Result<(), SyncError> {
        const CHECK: &[u8] = b"harbor-browser:vault-key-check:v1";
        let access_token = self.perform_handshake().await?;
        let mut request = SyncRequest {
            last_seen_revision: 0,
            push_items: Vec::new(),
        };
        let delta = self.exchange(&access_token, &request).await?;
        for envelope in &delta.pull_items {
            let plaintext = Zeroizing::new(decrypt_envelope(key, envelope)?);
            if envelope.id == VAULT_KEY_CHECK_ID {
                if plaintext.as_slice() != CHECK
                    || envelope.deleted
                    || envelope.item_type != ItemType::Setting
                {
                    return Err(SyncError::InvalidCredential(
                        "invalid vault key verification record",
                    ));
                }
                return Ok(());
            }
        }
        request.push_items.push(encrypt_envelope(
            key,
            VAULT_KEY_CHECK_ID,
            ItemType::Setting,
            false,
            CHECK,
            0,
        )?);
        let delta = self.exchange(&access_token, &request).await?;
        let marker = delta
            .pull_items
            .iter()
            .find(|record| record.id == VAULT_KEY_CHECK_ID)
            .ok_or(SyncError::InvalidCredential(
                "server did not return the vault key verification record",
            ))?;
        if decrypt_envelope(key, marker)? != CHECK
            || marker.deleted
            || marker.item_type != ItemType::Setting
        {
            return Err(SyncError::InvalidCredential(
                "invalid vault key verification record",
            ));
        }
        Ok(())
    }

    /// Encrypts local changes, authenticates fetched records, and commits the delta atomically.
    async fn sync_once(
        &self,
        store: &Mutex<LocalStore>,
        master_key: &MasterKey,
    ) -> Result<SyncBatch, SyncError> {
        let (dirty, last_seen_revision) = {
            let guard = store.lock().expect("store lock");
            (guard.fetch_dirty()?, guard.get_last_revision()?)
        };

        let access_token = self.perform_handshake().await?;

        let mut push_items = Vec::new();
        let mut payload_bytes = 0usize;
        for rec in &dirty {
            let size = rec.payload_json.len();
            if !push_items.is_empty() && payload_bytes + size > 512 * 1024 {
                break;
            }
            payload_bytes += size;
            let env = encrypt_envelope(
                master_key,
                rec.id,
                rec.item_type,
                rec.deleted,
                rec.payload_json.as_bytes(),
                rec.updated_at,
            )?;
            push_items.push(env);
        }

        let pushed = push_items.len();
        let delta = self
            .exchange(
                &access_token,
                &SyncRequest {
                    last_seen_revision,
                    push_items,
                },
            )
            .await?;
        if delta.current_revision < last_seen_revision {
            return Err(SyncError::InvalidCredential(
                "server revision moved backward",
            ));
        }

        let mut batch = SyncBatch {
            scripts_changed: false,
            filters_changed: false,
            more_to_upload: pushed < dirty.len(),
        };
        {
            let guard = store.lock().expect("store lock");
            guard.transaction(|store| {
                for env in delta.pull_items {
                    if env.id == VAULT_KEY_CHECK_ID {
                        continue;
                    }
                    let plaintext = Zeroizing::new(decrypt_envelope(master_key, &env)?);
                    let json = std::str::from_utf8(&plaintext)
                        .map_err(|_| SyncError::InvalidCredential("vault record is not UTF-8"))?;
                    if store.upsert_lww(
                        env.id,
                        env.item_type,
                        &json,
                        env.deleted,
                        false,
                        env.updated_at,
                    )? {
                        batch.scripts_changed |= env.item_type == ItemType::Userscript;
                        batch.filters_changed |= env.item_type == ItemType::AdblockConfig;
                    }
                }

                store.mark_clean(&dirty[..pushed])?;
                store.set_last_revision(delta.current_revision)?;
                Ok::<_, SyncError>(())
            })?;
        }

        Ok(batch)
    }
}

fn normalize_server_address(raw: &str) -> Result<String, SyncError> {
    let raw = raw.trim();
    let address = if raw.contains("://") {
        raw.to_string()
    } else if raw.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("https://[{raw}]")
    } else {
        format!("https://{raw}")
    };
    let invalid = || {
        SyncError::Address(
            "Enter the HTTPS address, hostname, or IP address of your Harbor server.",
        )
    };
    let url = reqwest::Url::parse(&address).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.has_host()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.starts_with(['/', '\\'])
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn configured_pin(raw: &str) -> Option<String> {
    let pin = raw.trim().replace(':', "");
    (!pin.is_empty()).then_some(pin)
}

#[derive(Serialize, Deserialize)]
struct SyncConfig {
    server_url: String,
    pin: Option<String>,
}

#[derive(Default)]
struct Session {
    unlocked: Option<(SyncClient, MasterKey)>,
    last_sync: Option<i64>,
    error: Option<String>,
}

pub struct SyncService {
    session: tokio::sync::Mutex<Session>,
    changed: tokio::sync::Notify,
}

impl SyncService {
    pub fn new() -> Self {
        Self {
            session: tokio::sync::Mutex::new(Session::default()),
            changed: tokio::sync::Notify::new(),
        }
    }

    pub fn changed(&self) {
        self.changed.notify_one();
    }

    fn config(store: &Mutex<LocalStore>) -> Result<Option<SyncConfig>, SyncError> {
        store
            .lock()
            .expect("store lock")
            .get_state("sync_config")?
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|_| SyncError::InvalidCredential("invalid saved server configuration"))
            })
            .transpose()
    }

    fn client(store: &Mutex<LocalStore>, config: &SyncConfig) -> Result<SyncClient, SyncError> {
        let profile_id = store.lock().expect("store lock").profile_id.clone();
        SyncClient::new(&config.server_url, config.pin.clone(), profile_id)
    }

    pub async fn check_server(
        &self,
        store: &Mutex<LocalStore>,
        server_url: String,
        pin: String,
    ) -> Result<String, SyncError> {
        let config = SyncConfig {
            server_url,
            pin: configured_pin(&pin),
        };
        let client = Self::client(store, &config)?;
        client.check_server().await?;
        Ok(client.server_url)
    }

    pub async fn status(&self, store: &Mutex<LocalStore>) -> Result<serde_json::Value, SyncError> {
        let session = self.session.lock().await;
        let config = Self::config(store)?;
        let paired = match &config {
            Some(config) => Self::client(store, config)?.is_paired(),
            None => false,
        };
        Ok(serde_json::json!({
            "server_url": config.map(|config| config.server_url),
            "paired": paired,
            "unlocked": session.unlocked.is_some(),
            "last_sync": session.last_sync,
            "error": session.error,
        }))
    }

    pub async fn pair(
        &self,
        store: &Mutex<LocalStore>,
        server_url: String,
        pin: String,
        device_name: String,
        code: Zeroizing<String>,
        passphrase: Zeroizing<String>,
    ) -> Result<(), SyncError> {
        let mut session = self.session.lock().await;
        if passphrase.is_empty() || code.trim().is_empty() || device_name.trim().is_empty() {
            return Err(SyncError::InvalidCredential(
                "device name, pairing code, and passphrase are required",
            ));
        }
        let mut config = SyncConfig {
            server_url,
            pin: configured_pin(&pin),
        };
        let client = Self::client(store, &config)?;
        config.server_url = client.server_url.clone();
        if let Some(previous) = Self::config(store)? {
            if Self::client(store, &previous)?.is_paired()
                || previous.server_url != config.server_url
            {
                return Err(SyncError::InvalidCredential(
                    "this profile already has a sync server",
                ));
            }
        }
        let config_json = serde_json::to_string(&config)
            .map_err(|_| SyncError::InvalidCredential("invalid server configuration"))?;
        client.pair_device(code.trim(), device_name.trim()).await?;
        if let Err(error) = store
            .lock()
            .expect("store lock")
            .set_state("sync_config", &config_json)
        {
            client.clear_credentials();
            return Err(error.into());
        }
        self.unlock_session(store, &mut session, client, passphrase)
            .await
    }

    pub async fn unlock(
        &self,
        store: &Mutex<LocalStore>,
        passphrase: Zeroizing<String>,
    ) -> Result<(), SyncError> {
        let mut session = self.session.lock().await;
        if session.unlocked.is_some() {
            return Ok(());
        }
        let config = Self::config(store)?
            .ok_or(SyncError::InvalidCredential("connect a sync server first"))?;
        self.unlock_session(
            store,
            &mut session,
            Self::client(store, &config)?,
            passphrase,
        )
        .await
    }

    async fn unlock_session(
        &self,
        store: &Mutex<LocalStore>,
        session: &mut Session,
        client: SyncClient,
        passphrase: Zeroizing<String>,
    ) -> Result<(), SyncError> {
        if passphrase.is_empty() {
            return Err(SyncError::InvalidCredential("enter the vault passphrase"));
        }
        let result = async {
            let salt = client.ensure_vault_salt().await?;
            let key = tokio::task::spawn_blocking(move || {
                MasterKey::derive(passphrase.as_bytes(), &salt)
            })
            .await
            .map_err(|_| SyncError::InvalidCredential("vault key derivation interrupted"))??;
            client.verify_vault_key(&key).await?;
            store
                .lock()
                .expect("store lock")
                .set_state("setup_complete", "1")?;
            Ok::<_, SyncError>(key)
        }
        .await;
        match result {
            Ok(key) => {
                session.unlocked = Some((client, key));
                session.error = None;
                self.changed();
                Ok(())
            }
            Err(error) => {
                if matches!(error, SyncError::DeviceRevoked) {
                    client.clear_credentials();
                }
                Err(error)
            }
        }
    }

    pub async fn sync_now(
        &self,
        runtime: &crate::cef_shell::HarborRuntime,
    ) -> Result<(), SyncError> {
        let mut session = self.session.lock().await;
        let (client, key) = session
            .unlocked
            .as_ref()
            .ok_or(SyncError::InvalidCredential("unlock the vault to sync"))?;
        let result = client.sync_once(&runtime.store, key).await;
        match result {
            Ok(batch) => {
                if batch.scripts_changed {
                    let store = runtime.store.lock().expect("store lock");
                    *runtime.scripts.lock().expect("scripts lock") =
                        crate::load_saved_userscripts(&store);
                }
                if batch.filters_changed {
                    runtime.filters_changed.notify_one();
                }
                if batch.more_to_upload {
                    self.changed();
                }
                session.last_sync = Some(crate::ipc_router::now_unix());
                session.error = None;
                Ok(())
            }
            Err(error) => {
                if matches!(error, SyncError::DeviceRevoked) {
                    client.clear_credentials();
                    session.unlocked = None;
                }
                session.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub async fn run(self: Arc<Self>, runtime: crate::cef_shell::HarborRuntime) {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {},
                _ = self.changed.notified() => tokio::time::sleep(Duration::from_secs(2)).await,
            }
            let unlocked = self.session.lock().await.unlocked.is_some();
            if unlocked {
                let _ = self.sync_now(&runtime).await;
            }
        }
    }
}
