//! I manage Harbor's MV3 extension subset here: package validation,
//! local storage in `LocalStore`, `Work`/`Personal` workspace isolation,
//! and content-script injection kept separate from `harbor://` IPC.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use flate2::read::DeflateDecoder;
use serde::{Deserialize, Serialize};
use shared_protocol::crypto::sha256_hex;
use uuid::Uuid;

use crate::storage::LocalStore;
use crate::tab_manager::SpaceKind;
use crate::userscripts::pattern_matches_url;

pub const MAX_EXTENSION_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_EXTENSION_FILES: usize = 256;
const EXTENSIONS_STATE_KEY: &str = "harbor:extensions:v1";

const BLOCKED_PERMISSIONS: &[&str] = &[
    "nativeMessaging",
    "debugger",
    "management",
    "proxy",
    "webRequestBlocking",
    "downloads",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentScriptRule {
    #[serde(default)]
    pub matches: Vec<String>,
    #[serde(default)]
    pub exclude_matches: Vec<String>,
    #[serde(default)]
    pub js: Vec<String>,
    #[serde(default)]
    pub css: Vec<String>,
    #[serde(default = "default_run_at")]
    pub run_at: String,
    #[serde(default)]
    pub all_frames: bool,
}

fn default_run_at() -> String {
    "document_idle".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestAction {
    #[serde(default)]
    pub default_popup: Option<String>,
    #[serde(default)]
    pub default_title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestOptionsUi {
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnrRuleResource {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestDnr {
    #[serde(default)]
    pub rule_resources: Vec<DnrRuleResource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestBackground {
    #[serde(default)]
    pub service_worker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestV3 {
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub host_permissions: Vec<String>,
    #[serde(default)]
    pub content_scripts: Vec<ContentScriptRule>,
    #[serde(default)]
    pub declarative_net_request: Option<ManifestDnr>,
    #[serde(default)]
    pub background: Option<ManifestBackground>,
    #[serde(default)]
    pub action: Option<ManifestAction>,
    #[serde(default)]
    pub options_page: Option<String>,
    #[serde(default)]
    pub options_ui: Option<ManifestOptionsUi>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub workspace: SpaceKind,
    pub source_path: String,
    pub digest: String,
    pub enabled: bool,
    pub permissions: Vec<String>,
    pub host_permissions: Vec<String>,
    pub content_scripts: Vec<ContentScriptRule>,
    #[serde(default)]
    pub dnr_rules: Vec<DnrRuleResource>,
    pub popup_path: Option<String>,
    pub options_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionReview {
    pub token: Uuid,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub workspace: SpaceKind,
    pub source_path: String,
    pub digest: String,
    pub permissions: Vec<String>,
    pub host_permissions: Vec<String>,
    pub content_scripts_count: usize,
    pub compatibility_notes: Vec<String>,
    pub popup_path: Option<String>,
    pub options_path: Option<String>,
    #[serde(skip_serializing)]
    pub manifest: ManifestV3,
    #[serde(skip_serializing)]
    pub files: BTreeMap<String, Vec<u8>>,
}

fn is_reparse_or_symlink(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    false
}

fn normalize_rel_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Err("Empty relative asset path in manifest.json.".into());
    }
    let path = Path::new(trimmed);
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            Component::Normal(seg) => {
                let s = seg
                    .to_str()
                    .ok_or_else(|| "Non-UTF8 path in extension manifest.".to_string())?;
                parts.push(s);
            }
            Component::CurDir => {}
            _ => return Err(format!("Unsafe path '{raw}' in extension manifest.")),
        }
    }
    if parts.is_empty() {
        return Err(format!("Invalid path '{raw}' in extension manifest."));
    }
    Ok(parts.join("/"))
}

fn collect_package_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let root_meta = fs::symlink_metadata(root)
        .map_err(|_| "Extension folder does not exist or cannot be read.".to_string())?;
    if !root_meta.is_dir() || is_reparse_or_symlink(&root_meta) {
        return Err(
            "Extension source must be a regular directory (not a symlink or reparse point).".into(),
        );
    }

    let mut files = BTreeMap::new();
    let mut total_bytes = 0u64;
    let mut stack = vec![(root.to_path_buf(), String::new())];

    while let Some((dir, prefix)) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("Failed reading extension directory: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed reading extension entry: {e}"))?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("Failed inspecting {}: {e}", path.display()))?;
            if is_reparse_or_symlink(&meta) {
                return Err(format!(
                    "Rejected symlink or reparse point '{}' inside extension package.",
                    path.display()
                ));
            }
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| "Non-UTF8 filename inside extension package.".to_string())?
                .to_string();
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };

            if meta.is_dir() {
                stack.push((path, rel));
            } else if meta.is_file() {
                if files.len() >= MAX_EXTENSION_FILES {
                    return Err(format!(
                        "Extension exceeds maximum of {MAX_EXTENSION_FILES} files."
                    ));
                }
                total_bytes = total_bytes.saturating_add(meta.len());
                if total_bytes > MAX_EXTENSION_BYTES {
                    return Err("Extension exceeds maximum package size of 10 MB.".into());
                }
                let bytes = fs::read(&path).map_err(|e| format!("Failed reading '{rel}': {e}"))?;
                files.insert(rel, bytes);
            } else {
                return Err(format!("Unsupported file type for '{rel}'."));
            }
        }
    }

    Ok(files)
}

pub fn review_unpacked(source_dir: &Path, workspace: SpaceKind) -> Result<ExtensionReview, String> {
    let files = collect_package_files(source_dir)?;
    review_files(
        files,
        source_dir.to_string_lossy().into_owned(),
        workspace,
        None,
    )
}

pub fn extract_cws_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let is_cws_id = |s: &str| s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b));
    if is_cws_id(trimmed) {
        return Some(trimmed.to_string());
    }
    let url = reqwest::Url::parse(trimmed).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "chromewebstore.google.com" && host != "chrome.google.com" {
        return None;
    }
    url.path_segments()?
        .rev()
        .find(|seg| is_cws_id(seg))
        .map(str::to_string)
}

fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    let slice: [u8; 2] = buf.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(slice))
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let slice: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(slice))
}

pub fn parse_crx_or_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let zip_bytes = if bytes.starts_with(b"Cr24") {
        let version = read_u32_le(bytes, 4).ok_or("Truncated CRX header.")?;
        let offset = match version {
            3 => {
                let header_len = read_u32_le(bytes, 8).ok_or("Truncated CRX3 header.")? as usize;
                12usize
                    .checked_add(header_len)
                    .ok_or("Invalid CRX3 header size.")?
            }
            2 => {
                let pub_len = read_u32_le(bytes, 8).ok_or("Truncated CRX2 header.")? as usize;
                let sig_len = read_u32_le(bytes, 12).ok_or("Truncated CRX2 header.")? as usize;
                16usize
                    .checked_add(pub_len)
                    .and_then(|v| v.checked_add(sig_len))
                    .ok_or("Invalid CRX2 header size.")?
            }
            other => return Err(format!("Unsupported CRX version {other}.")),
        };
        bytes.get(offset..).ok_or("CRX archive payload missing.")?
    } else {
        bytes
    };

    if !zip_bytes.starts_with(b"PK\x03\x04") {
        return Err("Package is not a valid CRX or ZIP archive.".into());
    }

    // Locate End of Central Directory (PK\x05\x06)
    let search_start = zip_bytes.len().saturating_sub(65_557);
    let eocd_pos = (search_start..=zip_bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| &zip_bytes[i..i + 4] == b"PK\x05\x06")
        .ok_or("Missing ZIP End of Central Directory.")?;
    let entry_count = read_u16_le(zip_bytes, eocd_pos + 10).ok_or("Invalid ZIP EOCD.")? as usize;
    let cd_offset = read_u32_le(zip_bytes, eocd_pos + 16).ok_or("Invalid ZIP CD offset.")? as usize;
    if entry_count > MAX_EXTENSION_FILES * 2 {
        return Err(format!(
            "Extension exceeds maximum of {MAX_EXTENSION_FILES} files."
        ));
    }

    let mut files = BTreeMap::new();
    let mut total_bytes = 0u64;
    let mut pos = cd_offset;

    for _ in 0..entry_count {
        if zip_bytes.get(pos..pos + 4) != Some(b"PK\x01\x02") {
            return Err("Corrupt ZIP central directory entry.".into());
        }
        let method = read_u16_le(zip_bytes, pos + 10).ok_or("Invalid ZIP method.")?;
        let comp_size =
            read_u32_le(zip_bytes, pos + 20).ok_or("Invalid ZIP compressed size.")? as usize;
        let name_len = read_u16_le(zip_bytes, pos + 28).ok_or("Invalid ZIP name length.")? as usize;
        let extra_len =
            read_u16_le(zip_bytes, pos + 30).ok_or("Invalid ZIP extra length.")? as usize;
        let comment_len =
            read_u16_le(zip_bytes, pos + 32).ok_or("Invalid ZIP comment length.")? as usize;
        let local_off =
            read_u32_le(zip_bytes, pos + 42).ok_or("Invalid ZIP local offset.")? as usize;

        let name_bytes = zip_bytes
            .get(pos + 46..pos + 46 + name_len)
            .ok_or("Truncated ZIP filename.")?;
        let raw_name = std::str::from_utf8(name_bytes)
            .map_err(|_| "Non-UTF8 filename in extension archive.".to_string())?;
        pos = pos + 46 + name_len + extra_len + comment_len;

        if raw_name.ends_with('/') {
            continue;
        }
        let rel = normalize_rel_path(raw_name)?;
        if files.len() >= MAX_EXTENSION_FILES {
            return Err(format!(
                "Extension exceeds maximum of {MAX_EXTENSION_FILES} files."
            ));
        }

        if zip_bytes.get(local_off..local_off + 4) != Some(b"PK\x03\x04") {
            return Err("Corrupt ZIP local file header.".into());
        }
        let l_name_len =
            read_u16_le(zip_bytes, local_off + 26).ok_or("Invalid local header.")? as usize;
        let l_extra_len =
            read_u16_le(zip_bytes, local_off + 28).ok_or("Invalid local header.")? as usize;
        let data_start = local_off + 30 + l_name_len + l_extra_len;
        let comp_slice = zip_bytes
            .get(data_start..data_start + comp_size)
            .ok_or("Truncated ZIP entry data.")?;

        let remaining_budget = MAX_EXTENSION_BYTES.saturating_sub(total_bytes);
        let data = match method {
            0 => {
                if (comp_slice.len() as u64) > remaining_budget {
                    return Err("Extension exceeds maximum package size of 10 MB.".into());
                }
                comp_slice.to_vec()
            }
            8 => {
                let mut decoder = DeflateDecoder::new(comp_slice).take(remaining_budget + 1);
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| format!("Failed decompressing '{rel}': {e}"))?;
                if (out.len() as u64) > remaining_budget {
                    return Err("Extension exceeds maximum package size of 10 MB.".into());
                }
                out
            }
            other => {
                return Err(format!(
                    "Unsupported ZIP compression method {other} for '{rel}'."
                ));
            }
        };
        total_bytes = total_bytes.saturating_add(data.len() as u64);
        files.insert(rel, data);
    }

    Ok(files)
}

pub async fn review_extension_input(
    input: &str,
    workspace: SpaceKind,
) -> Result<ExtensionReview, String> {
    let trimmed = input.trim();
    if let Some(ext_id) = extract_cws_id(trimmed) {
        let download_url = format!(
            "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=134.0.0.0&acceptformat=crx2,crx3&x=id%3D{ext_id}%26uc"
        );
        let client = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed creating HTTP client: {e}"))?;
        let resp = client
            .get(&download_url)
            .send()
            .await
            .map_err(|e| format!("Failed downloading extension from Chrome Web Store: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "Chrome Web Store download failed with status {}.",
                resp.status()
            ));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Failed reading Chrome Web Store package: {e}"))?;
        if bytes.len() as u64 > MAX_EXTENSION_BYTES * 2 {
            return Err("Downloaded CRX package exceeds maximum allowed size.".into());
        }
        let files = parse_crx_or_zip(&bytes)?;
        return review_files(
            files,
            format!("chrome-web-store:{ext_id}"),
            workspace,
            Some(ext_id),
        );
    }

    let path = Path::new(trimmed);
    if path.is_file() {
        let meta = fs::symlink_metadata(path)
            .map_err(|e| format!("Failed inspecting package file: {e}"))?;
        if is_reparse_or_symlink(&meta) {
            return Err("Extension package cannot be a symlink or reparse point.".into());
        }
        if meta.len() > MAX_EXTENSION_BYTES * 2 {
            return Err("Extension archive exceeds maximum allowed size.".into());
        }
        let bytes = fs::read(path).map_err(|e| format!("Failed reading archive: {e}"))?;
        let files = parse_crx_or_zip(&bytes)?;
        return review_files(files, path.to_string_lossy().into_owned(), workspace, None);
    }

    review_unpacked(path, workspace)
}

fn review_files(
    files: BTreeMap<String, Vec<u8>>,
    source_path: String,
    workspace: SpaceKind,
    preferred_id: Option<String>,
) -> Result<ExtensionReview, String> {
    let manifest_bytes = files
        .get("manifest.json")
        .ok_or_else(|| "Missing manifest.json in extension folder.".to_string())?;
    let mut manifest: ManifestV3 = serde_json::from_slice(manifest_bytes)
        .map_err(|e| format!("Invalid manifest.json: {e}"))?;

    if manifest.manifest_version != 3 {
        return Err(
            "Only Chrome Manifest V3 extensions (manifest_version: 3) are supported.".into(),
        );
    }
    manifest.name = manifest.name.trim().to_string();
    manifest.version = manifest.version.trim().to_string();
    if manifest.name.is_empty() || manifest.version.is_empty() {
        return Err("manifest.json must declare a non-empty name and version.".into());
    }

    const SUPPORTED_V1_PERMS: &[&str] = &[
        "storage",
        "activeTab",
        "scripting",
        "declarativeNetRequest",
        "declarativeNetRequestWithHostAccess",
        "unlimitedStorage",
    ];
    let mut unsupported_perms = Vec::new();
    for perm in &manifest.permissions {
        if BLOCKED_PERMISSIONS.contains(&perm.as_str()) {
            return Err(format!(
                "Permission '{perm}' is not supported in Harbor v1 extensions."
            ));
        }
        if !SUPPORTED_V1_PERMS.contains(&perm.as_str()) {
            unsupported_perms.push(perm.as_str());
        }
    }

    let mut compatibility_notes = Vec::new();
    if manifest
        .background
        .as_ref()
        .and_then(|b| b.service_worker.as_ref())
        .is_some()
    {
        compatibility_notes.push(
            "Declares a background service worker (not executed in Harbor v1; content scripts, popups, and declarativeNetRequest rules will still run)."
                .into(),
        );
    }
    if !unsupported_perms.is_empty() {
        compatibility_notes.push(format!(
            "Uses APIs outside Harbor's v1 MV3 subset: {}. Features depending on those APIs may be limited.",
            unsupported_perms.join(", ")
        ));
    }

    for cs in &mut manifest.content_scripts {
        for js in &mut cs.js {
            let norm = normalize_rel_path(js)?;
            if !files.contains_key(&norm) {
                return Err(format!(
                    "Missing content script '{norm}' referenced by manifest.json."
                ));
            }
            *js = norm;
        }
        for css in &mut cs.css {
            let norm = normalize_rel_path(css)?;
            if !files.contains_key(&norm) {
                return Err(format!(
                    "Missing stylesheet '{norm}' referenced by manifest.json."
                ));
            }
            *css = norm;
        }
    }

    if let Some(dnr) = &mut manifest.declarative_net_request {
        for res in &mut dnr.rule_resources {
            let norm = normalize_rel_path(&res.path)?;
            let raw = files
                .get(&norm)
                .ok_or_else(|| format!("Missing declarativeNetRequest ruleset '{norm}'."))?;
            let parsed: Vec<DnrRule> = serde_json::from_slice(raw)
                .map_err(|e| format!("Invalid declarativeNetRequest ruleset '{norm}': {e}"))?;
            let _ = parsed.len();
            res.path = norm;
        }
    }

    let popup_path = match manifest
        .action
        .as_ref()
        .and_then(|a| a.default_popup.as_ref())
    {
        Some(p) => {
            let norm = normalize_rel_path(p)?;
            if !files.contains_key(&norm) {
                return Err(format!(
                    "Missing action popup '{norm}' referenced by manifest.json."
                ));
            }
            Some(norm)
        }
        None => None,
    };

    let options_raw = manifest
        .options_page
        .as_ref()
        .or_else(|| manifest.options_ui.as_ref().and_then(|o| o.page.as_ref()));
    let options_path = match options_raw {
        Some(p) => {
            let norm = normalize_rel_path(p)?;
            if !files.contains_key(&norm) {
                return Err(format!(
                    "Missing options page '{norm}' referenced by manifest.json."
                ));
            }
            Some(norm)
        }
        None => None,
    };

    // Deterministic SHA-256 package digest across all sorted files
    let mut digest_input = Vec::new();
    for (rel, bytes) in &files {
        digest_input.extend_from_slice(rel.as_bytes());
        digest_input.push(0);
        digest_input.extend_from_slice(bytes);
        digest_input.push(0);
    }
    let digest = sha256_hex(&digest_input);
    let id = preferred_id.unwrap_or_else(|| {
        let slug: String = manifest
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .take(24)
            .collect();
        format!("{}-{}", slug.trim_matches('-'), &digest[..12])
    });

    Ok(ExtensionReview {
        token: Uuid::new_v4(),
        id,
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone().unwrap_or_default(),
        workspace,
        source_path,
        digest,
        permissions: manifest.permissions.clone(),
        host_permissions: manifest.host_permissions.clone(),
        content_scripts_count: manifest.content_scripts.len(),
        compatibility_notes,
        popup_path,
        options_path,
        manifest,
        files,
    })
}

pub fn list_extensions(store: &LocalStore) -> Vec<InstalledExtension> {
    store
        .get_state(EXTENSIONS_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_extensions(store: &LocalStore, list: &[InstalledExtension]) -> Result<(), String> {
    let json = serde_json::to_string(list).map_err(|e| e.to_string())?;
    store
        .set_state(EXTENSIONS_STATE_KEY, &json)
        .map_err(|e| e.to_string())
}

pub fn extension_dir(profile_dir: &Path, id: &str, version: &str) -> PathBuf {
    profile_dir.join("extensions").join(id).join(version)
}

pub fn install_reviewed(
    store: &LocalStore,
    profile_dir: &Path,
    review: &ExtensionReview,
) -> Result<InstalledExtension, String> {
    let target_dir = extension_dir(profile_dir, &review.id, &review.version);
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)
            .map_err(|e| format!("Failed replacing existing extension directory: {e}"))?;
    }
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Failed creating extension directory: {e}"))?;

    for (rel, bytes) in &review.files {
        let dest = target_dir.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed creating asset parent directory: {e}"))?;
        }
        fs::write(&dest, bytes)
            .map_err(|e| format!("Failed writing extension asset '{rel}': {e}"))?;
    }

    let record = InstalledExtension {
        id: review.id.clone(),
        name: review.name.clone(),
        version: review.version.clone(),
        description: review.description.clone(),
        workspace: review.workspace,
        source_path: review.source_path.clone(),
        digest: review.digest.clone(),
        enabled: true,
        permissions: review.permissions.clone(),
        host_permissions: review.host_permissions.clone(),
        content_scripts: review.manifest.content_scripts.clone(),
        dnr_rules: review
            .manifest
            .declarative_net_request
            .as_ref()
            .map(|d| d.rule_resources.clone())
            .unwrap_or_default(),
        popup_path: review.popup_path.clone(),
        options_path: review.options_path.clone(),
    };

    let mut list = list_extensions(store);
    list.retain(|e| e.id != record.id);
    list.push(record.clone());
    save_extensions(store, &list)?;
    Ok(record)
}

pub fn set_extension_enabled(store: &LocalStore, id: &str, enabled: bool) -> Result<(), String> {
    let mut list = list_extensions(store);
    let ext = list
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| "Extension not found.".to_string())?;
    ext.enabled = enabled;
    save_extensions(store, &list)
}

pub fn remove_extension(store: &LocalStore, profile_dir: &Path, id: &str) -> Result<(), String> {
    let mut list = list_extensions(store);
    list.retain(|e| e.id != id);
    save_extensions(store, &list)?;
    let ext_root = profile_dir.join("extensions").join(id);
    if ext_root.exists() {
        let _ = fs::remove_dir_all(&ext_root);
    }
    Ok(())
}

pub fn matching_content_scripts(
    extensions: &[InstalledExtension],
    profile_dir: &Path,
    workspace: SpaceKind,
    url: &str,
    early: bool,
) -> Vec<String> {
    let mut scripts = Vec::new();
    for ext in extensions {
        if !ext.enabled || ext.workspace != workspace {
            continue;
        }
        let base_dir = extension_dir(profile_dir, &ext.id, &ext.version);
        for rule in &ext.content_scripts {
            let is_early = rule.run_at == "document_start";
            if is_early != early {
                continue;
            }
            let excluded = rule
                .exclude_matches
                .iter()
                .any(|p| pattern_matches_url(p, url));
            if excluded {
                continue;
            }
            if !rule.matches.iter().any(|p| pattern_matches_url(p, url)) {
                continue;
            }
            for css_rel in &rule.css {
                if let Ok(css) = fs::read_to_string(base_dir.join(css_rel)) {
                    scripts.push(format!(
                        "(function(){{const s=document.createElement('style');s.textContent={:?};(document.head||document.documentElement)?.appendChild(s);}})();",
                        css
                    ));
                }
            }
            for js_rel in &rule.js {
                if let Ok(code) = fs::read_to_string(base_dir.join(js_rel)) {
                    scripts.push(wrap_extension_script(ext, &code));
                }
            }
        }
    }
    scripts
}

pub fn wrap_extension_script(ext: &InstalledExtension, code: &str) -> String {
    let prefix = format!("__harbor_ext:{}:{}:", ext.workspace.as_str(), ext.id);
    let manifest_json = serde_json::json!({
        "manifest_version": 3,
        "name": ext.name,
        "version": ext.version,
    });
    format!(
        "(function(){{
const _extPrefix={prefix:?};
const _extManifest={manifest};
const chrome={{
  runtime:{{
    id:{id:?},
    getManifest:()=>_extManifest,
    getURL:(p)=>'harbor://extension-asset/'+{id:?}+'/'+{ver:?}+'/'+String(p||'').replace(/^\\/+/, '')
  }},
  storage:{{
    local:{{
      get:(keys,cb)=>new Promise((resolve)=>{{
        const out={{}};
        const read=(k)=>{{try{{const r=localStorage.getItem(_extPrefix+k);return r===null?undefined:JSON.parse(r);}}catch{{return undefined;}}}};
        if(typeof keys==='string'){{const v=read(keys);if(v!==undefined)out[keys]=v;}}
        else if(Array.isArray(keys)){{for(const k of keys){{const v=read(k);if(v!==undefined)out[k]=v;}}}}
        else if(keys&&typeof keys==='object'){{for(const [k,d] of Object.entries(keys)){{const v=read(k);out[k]=v===undefined?d:v;}}}}
        else{{try{{for(let i=0;i<localStorage.length;i++){{const k=localStorage.key(i);if(k&&k.startsWith(_extPrefix))out[k.slice(_extPrefix.length)]=JSON.parse(localStorage.getItem(k));}}}}catch{{}}}}
        if(cb)cb(out);resolve(out);
      }}),
      set:(items,cb)=>new Promise((resolve)=>{{
        try{{for(const [k,v] of Object.entries(items||{{}}))localStorage.setItem(_extPrefix+k,JSON.stringify(v));}}catch{{}}
        if(cb)cb();resolve();
      }}),
      remove:(keys,cb)=>new Promise((resolve)=>{{
        const arr=Array.isArray(keys)?keys:[keys];
        try{{for(const k of arr)localStorage.removeItem(_extPrefix+k);}}catch{{}}
        if(cb)cb();resolve();
      }}),
      clear:(cb)=>new Promise((resolve)=>{{
        try{{const rm=[];for(let i=0;i<localStorage.length;i++){{const k=localStorage.key(i);if(k&&k.startsWith(_extPrefix))rm.push(k);}}rm.forEach(k=>localStorage.removeItem(k));}}catch{{}}
        if(cb)cb();resolve();
      }})
    }}
  }}
}};
try{{
{code}
}}catch(e){{console.error('[Harbor Extension Error]',{name:?},e);}}
}})();",
        prefix = prefix,
        manifest = manifest_json,
        id = ext.id,
        ver = ext.version,
        name = ext.name,
        code = code,
    )
}

#[derive(Debug, Deserialize)]
struct DnrRule {
    action: DnrAction,
    condition: DnrCondition,
}

#[derive(Debug, Deserialize)]
struct DnrAction {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DnrCondition {
    #[serde(default)]
    url_filter: Option<String>,
    #[serde(default)]
    initiator_domains: Vec<String>,
    #[serde(default)]
    domains: Vec<String>,
    #[serde(default)]
    resource_types: Vec<String>,
}

pub fn compile_extension_dnr_rules(
    extensions: &[InstalledExtension],
    profile_dir: &Path,
    workspace: SpaceKind,
) -> Vec<String> {
    let mut compiled = Vec::new();
    for ext in extensions {
        if !ext.enabled || ext.workspace != workspace {
            continue;
        }
        let base_dir = extension_dir(profile_dir, &ext.id, &ext.version);
        for res in &ext.dnr_rules {
            if !res.enabled {
                continue;
            }
            let Ok(raw) = fs::read(base_dir.join(&res.path)) else {
                continue;
            };
            let Ok(rules) = serde_json::from_slice::<Vec<DnrRule>>(&raw) else {
                continue;
            };
            for rule in rules {
                let prefix = match rule.action.kind.as_str() {
                    "block" => "",
                    "allow" => "@@",
                    _ => continue,
                };
                let filter = rule.condition.url_filter.as_deref().unwrap_or("*").trim();
                if filter.is_empty() {
                    continue;
                }
                let mut opts = Vec::new();
                for rtype in &rule.condition.resource_types {
                    let mapped = match rtype.as_str() {
                        "main_frame" => "document",
                        "sub_frame" => "subdocument",
                        "xmlhttprequest" => "xmlhttprequest",
                        "script" => "script",
                        "image" => "image",
                        "stylesheet" => "stylesheet",
                        "font" => "font",
                        "media" => "media",
                        "websocket" => "websocket",
                        _ => continue,
                    };
                    opts.push(mapped.to_string());
                }
                let mut domains = rule.condition.initiator_domains;
                domains.extend(rule.condition.domains);
                if !domains.is_empty() {
                    opts.push(format!("domain={}", domains.join("|")));
                }
                if opts.is_empty() {
                    compiled.push(format!("{prefix}{filter}"));
                } else {
                    compiled.push(format!("{prefix}{filter}${}", opts.join(",")));
                }
            }
        }
    }
    compiled
}

pub fn resolve_extension_asset(
    extensions: &[InstalledExtension],
    profile_dir: &Path,
    url: &str,
) -> Option<(&'static str, Vec<u8>)> {
    let rest = url.strip_prefix("harbor://extension-asset/")?;
    let mut parts = rest.splitn(3, '/');
    let id = parts.next()?;
    let version = parts.next()?;
    let rel = normalize_rel_path(parts.next()?).ok()?;
    let ext = extensions
        .iter()
        .find(|e| e.id == id && e.version == version && e.enabled)?;
    let path = extension_dir(profile_dir, &ext.id, &ext.version).join(&rel);
    let bytes = fs::read(&path).ok()?;
    let mime = match Path::new(&rel).extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    Some((mime, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("harbor-ext-{name}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reviews_installs_isolates_and_removes_unpacked_mv3_extension() {
        let pkg = temp_dir("pkg");
        let profile = temp_dir("profile");
        fs::write(
            pkg.join("manifest.json"),
            r#"{
                "manifest_version": 3,
                "name": "Example Marker",
                "version": "1.2.0",
                "permissions": ["storage"],
                "host_permissions": ["https://example.com/*"],
                "content_scripts": [{
                    "matches": ["https://*.example.com/*"],
                    "js": ["content.js"],
                    "run_at": "document_end"
                }]
            }"#,
        )
        .unwrap();
        fs::write(
            pkg.join("content.js"),
            "document.body.dataset.harborExt='1';",
        )
        .unwrap();

        let store = LocalStore::test_store(&profile.join("local.db")).unwrap();
        let review = review_unpacked(&pkg, SpaceKind::Work).unwrap();
        assert_eq!(review.name, "Example Marker");

        let installed = install_reviewed(&store, &profile, &review).unwrap();
        assert_eq!(list_extensions(&store).len(), 1);

        let work_scripts = matching_content_scripts(
            &list_extensions(&store),
            &profile,
            SpaceKind::Work,
            "https://example.com/",
            false,
        );
        assert_eq!(work_scripts.len(), 1);
        assert!(work_scripts[0].contains("const chrome={"));
        assert!(work_scripts[0].contains("storage:{"));

        // Workspace boundary: Personal workspace must not receive Work extensions
        let personal_scripts = matching_content_scripts(
            &list_extensions(&store),
            &profile,
            SpaceKind::Personal,
            "https://example.com/",
            false,
        );
        assert!(personal_scripts.is_empty());

        // Disable stops injection
        set_extension_enabled(&store, &installed.id, false).unwrap();
        let disabled_scripts = matching_content_scripts(
            &list_extensions(&store),
            &profile,
            SpaceKind::Work,
            "https://example.com/",
            false,
        );
        assert!(disabled_scripts.is_empty());

        // Removal cleans registry and files
        remove_extension(&store, &profile, &installed.id).unwrap();
        assert!(list_extensions(&store).is_empty());
        assert!(!profile.join("extensions").join(&installed.id).exists());

        let _ = fs::remove_dir_all(&pkg);
        let _ = fs::remove_dir_all(&profile);
    }

    #[test]
    fn rejects_blocked_permissions_and_path_traversal() {
        let pkg = temp_dir("bad");
        fs::write(
            pkg.join("manifest.json"),
            r#"{
                "manifest_version": 3,
                "name": "Bad Ext",
                "version": "1.0.0",
                "permissions": ["nativeMessaging"]
            }"#,
        )
        .unwrap();
        assert!(review_unpacked(&pkg, SpaceKind::Work).is_err());

        fs::write(
            pkg.join("manifest.json"),
            r#"{
                "manifest_version": 3,
                "name": "Traversal Ext",
                "version": "1.0.0",
                "content_scripts": [{"matches": ["https://example.com/*"], "js": ["../secret.js"]}]
            }"#,
        )
        .unwrap();
        assert!(review_unpacked(&pkg, SpaceKind::Work).is_err());

        let _ = fs::remove_dir_all(&pkg);
    }

    #[test]
    fn compiles_declarative_net_request_rules_into_adblock_manager() {
        let pkg = temp_dir("dnr");
        let profile = temp_dir("dnr-profile");
        fs::write(
            pkg.join("manifest.json"),
            r#"{
                "manifest_version": 3,
                "name": "DNR Blocker",
                "version": "1.0.0",
                "permissions": ["declarativeNetRequest"],
                "declarative_net_request": {
                    "rule_resources": [{
                        "id": "ruleset_1",
                        "enabled": true,
                        "path": "rules.json"
                    }]
                }
            }"#,
        )
        .unwrap();
        fs::write(
            pkg.join("rules.json"),
            r#"[{
                "id": 1,
                "priority": 1,
                "action": { "type": "block" },
                "condition": { "urlFilter": "||tracker.example.net^", "resourceTypes": ["script"] }
            }]"#,
        )
        .unwrap();

        let store = LocalStore::test_store(&profile.join("local.db")).unwrap();
        let review = review_unpacked(&pkg, SpaceKind::Work).unwrap();
        install_reviewed(&store, &profile, &review).unwrap();
        let exts = list_extensions(&store);
        let rules = compile_extension_dnr_rules(&exts, &profile, SpaceKind::Work);
        assert_eq!(rules, vec!["||tracker.example.net^$script"]);

        let mut mgr = crate::adblock_engine::AdblockManager::from_rules(&[], vec![]);
        mgr.set_extension_rules(&rules);
        assert!(mgr.should_block(
            "https://tracker.example.net/a.js",
            "https://example.com/",
            "script"
        ));

        let _ = fs::remove_dir_all(&pkg);
        let _ = fs::remove_dir_all(&profile);
    }

    #[test]
    fn parses_crx3_zip_cws_urls_and_reports_compatibility_notes() {
        assert_eq!(
            extract_cws_id(
                "https://chromewebstore.google.com/detail/example/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            extract_cws_id("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert!(extract_cws_id("not-an-id").is_none());

        // Build a minimal valid ZIP containing manifest.json in STORE mode wrapped in a Cr24 v3 header
        let manifest = br#"{"manifest_version":3,"name":"CRX Ext","version":"2.0.0","permissions":["storage","tabs"],"background":{"service_worker":"bg.js"}}"#;
        let name = b"manifest.json";
        let mut zip = Vec::new();
        // Local file header
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20u16.to_le_bytes()); // ver
        zip.extend_from_slice(&0u16.to_le_bytes()); // flags
        zip.extend_from_slice(&0u16.to_le_bytes()); // method STORE
        zip.extend_from_slice(&0u16.to_le_bytes()); // time
        zip.extend_from_slice(&0u16.to_le_bytes()); // date
        zip.extend_from_slice(&0u32.to_le_bytes()); // crc
        zip.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(manifest);

        let cd_offset = zip.len() as u32;
        // Central directory header
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // date
        zip.extend_from_slice(&0u32.to_le_bytes()); // crc
        zip.extend_from_slice(&(manifest.len() as u32).to_le_bytes()); // comp
        zip.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes()); // local offset 0
        zip.extend_from_slice(name);
        let cd_size = (zip.len() as u32) - cd_offset;

        // End of Central Directory
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&cd_size.to_le_bytes());
        zip.extend_from_slice(&cd_offset.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());

        let mut crx = Vec::new();
        crx.extend_from_slice(b"Cr24");
        crx.extend_from_slice(&3u32.to_le_bytes());
        crx.extend_from_slice(&4u32.to_le_bytes()); // 4-byte dummy proto header
        crx.extend_from_slice(&[0u8; 4]);
        crx.extend_from_slice(&zip);

        let files = parse_crx_or_zip(&crx).unwrap();
        let review =
            review_files(files, "chrome-web-store:test".into(), SpaceKind::Work, None).unwrap();
        assert_eq!(review.name, "CRX Ext");
        assert_eq!(review.compatibility_notes.len(), 2);
        assert!(review.compatibility_notes[0].contains("service worker"));
        assert!(review.compatibility_notes[1].contains("tabs"));
    }
}
