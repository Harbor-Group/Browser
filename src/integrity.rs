//! Runtime binary integrity verifier for Harbor's CEF runtime.
//!
//! Protects against DLL hijacking / swapping (e.g., replacing `libcef.dll` or `chrome_elf.dll`
//! to intercept browsing traffic or credentials) by verifying cryptographic SHA-256 digests
//! before CEF initializes.
//!
//! Note on Spotify CEF builds: Spotify's stock automated CEF builds ship unsigned DLLs with
//! CDN checksums. `scripts/bundle-windows.ps1` verifies the CDN checksum at build time and
//! writes `cef_integrity.json` with SHA-256 hashes of all critical CEF binaries. When you
//! code-sign Harbor for release, you can also enable Authenticode thumbprint verification.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use shared_protocol::crypto::sha256_hex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IntegrityError {
    #[error("failed to locate executable directory: {0}")]
    ExeDir(std::io::Error),
    #[error("missing integrity manifest at {0}")]
    MissingManifest(PathBuf),
    #[error("invalid integrity manifest: {0}")]
    InvalidManifest(#[from] serde_json::Error),
    #[error("missing required CEF binary: {0}")]
    MissingBinary(String),
    #[error("TAMPER DETECTED: {file} SHA-256 mismatch (expected {expected}, got {actual})")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("failed to read {file}: {source}")]
    Io {
        file: String,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CefIntegrityManifest {
    pub cef_version: String,
    /// Map of relative filename (e.g., "libcef.dll") -> lowercase hex SHA-256 digest.
    pub files: HashMap<String, String>,
    /// Optional Authenticode signer certificate SHA-1/SHA-256 thumbprint for signed releases.
    pub expected_signer_thumbprint: Option<String>,
}

/// Verifies every file listed in `cef_integrity.json` located beside `harbor.exe`.
///
/// In `cef-runtime` release builds, any missing manifest or hash mismatch aborts startup
/// immediately before CEF is initialized.
pub fn verify_cef_binaries() -> Result<(), IntegrityError> {
    let exe_path = std::env::current_exe().map_err(IntegrityError::ExeDir)?;
    let exe_dir = exe_path
        .parent()
        .ok_or_else(|| IntegrityError::ExeDir(std::io::Error::other("no parent directory")))?;

    verify_cef_binaries_in_dir(exe_dir)
}

pub fn verify_cef_binaries_in_dir(dir: &Path) -> Result<(), IntegrityError> {
    let manifest_path = dir.join("cef_integrity.json");
    if !manifest_path.exists() {
        return Err(IntegrityError::MissingManifest(manifest_path));
    }

    let raw = fs::read_to_string(&manifest_path).map_err(|source| IntegrityError::Io {
        file: "cef_integrity.json".to_string(),
        source,
    })?;
    let clean_raw = raw.trim_start_matches('\u{feff}');
    let manifest: CefIntegrityManifest = serde_json::from_str(clean_raw)?;

    for (filename, expected_hash) in &manifest.files {
        let target = dir.join(filename);
        if !target.exists() {
            return Err(IntegrityError::MissingBinary(filename.clone()));
        }
        let bytes = fs::read(&target).map_err(|source| IntegrityError::Io {
            file: filename.clone(),
            source,
        })?;
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(expected_hash) {
            return Err(IntegrityError::HashMismatch {
                file: filename.clone(),
                expected: expected_hash.clone(),
                actual,
            });
        }
    }

    tracing::info!(
        cef_version = %manifest.cef_version,
        verified_files = manifest.files.len(),
        "CEF binary integrity check passed"
    );
    Ok(())
}
