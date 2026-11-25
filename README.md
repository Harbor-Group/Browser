# Harbor Browser

[![Harbor Browser CI & Release](https://github.com/Harbor-Group/Browser/actions/workflows/ci-release.yml/badge.svg)](https://github.com/Harbor-Group/Browser/actions/workflows/ci-release.yml)

Desktop browser client for Harbor, built with Rust and Chromium Embedded Framework (CEF).

## Technical Architecture

1. Runtime: Rust (`edition = "2024"`) embedding CEF 156 (`cef-156`, Alloy runtime style) with a `harbor.dll` library target and `harbor.exe` binary target.
2. Window Layout: Frameless top-level CEF window with a 48px `harbor://chrome` toolbar, a multi-view content panel supporting Split View, and overlay popovers for commands and menus.
3. Background Tabs: Per-tab `BrowserView` pool using `set_visible(0)` and `was_hidden(1)` for non-rendering background tabs, configurable LRU cap, inactivity sleep timer, and audio playback detection via `harbor.media`.
4. Internal Pages: Embedded `harbor://` scheme handler serving `harbor://home`, `harbor://settings`, `harbor://bookmarks`, `harbor://history`, `harbor://logins`, `harbor://adblock`, `harbor://scripts`, `harbor://extensions`, and `harbor://server`.
5. Content Filtering: Brave `adblock` engine with binary filter cache, custom filter lists, per-site allowlisting, and cosmetic CSS injection.
6. Extensions & User Scripts: Manifest V3 subset (`content_scripts`, `declarativeNetRequest`, `action.default_popup`, `storage`, `tabs`, `runtime`) with CRX3/ZIP/unpacked/Chrome Web Store import, plus native `@match`/`@include`/`@exclude` UserScript injection.
7. Storage & Sync: Encrypted local SQLite storage (`rusqlite` 0.43) keyed via OS credential store, end-to-end encrypted sync (`XChaCha20-Poly1305`, `Argon2id`, `Ed25519`) over TLS 1.3 with certificate pinning via `shared-protocol`.
8. Developer Tools: Native Chromium DevTools toggled via `F12`, `Ctrl+Shift+I`, or the Harbor menu.

## Requirements

1. Rust toolchain (stable, 2024 edition).
2. Git submodule `shared-protocol` initialized.
3. Windows 10/11 SDK (`rc.exe`) for resource compilation and CEF 156 runtime binaries for full `cef-runtime` builds.

## Build and Test

```bash
git submodule update --init --recursive
cargo test --lib
cargo clippy --lib -- -D warnings -A dead_code
```

To build with the CEF runtime enabled:

```bash
cargo build --release --features cef-runtime
```

## Continuous Integration

1. `quality`: Runs `cargo fmt --all -- --check`, `cargo clippy --lib -- -D warnings -A dead_code`, and `cargo test --lib` on `windows-latest`.
2. `security`: Runs `cargo audit` on `ubuntu-latest`.
3. `build-and-release`: Builds release binaries on `windows-latest`, uploads `harbor-browser-windows-x86_64.zip` with SHA-256 checksum, and publishes a GitHub Release on `v*` tags.
