use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bzip2::read::BzDecoder;
use serde::Deserialize;
use sha1_smol::Sha1;
use tar::Archive;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ICC_PROGRESS_CLASS, INITCOMMONCONTROLSEX, InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32,
    PBS_SMOOTH, PROGRESS_CLASSW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, IDCANCEL, IDOK, MB_ICONERROR, MB_OK, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SendMessageW,
    SetWindowTextW, ShowWindow, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY,
    WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_SYSMENU, WS_VISIBLE,
};

const CEF_VERSION: &str = "156.0.31";
const DEFAULT_CDN: &str = "https://cef-builds.spotifycdn.com";
const FALLBACK_ARCHIVE: &str =
    "cef_binary_156.0.31+ge603d97+chromium-156.0.8078.100_windows64_minimal.tar.bz2";
const FALLBACK_SHA1: &str = "d179fb251141d6c30992a1775db80a6a879852ba";

const WM_APP_STATUS: u32 = WM_APP + 1;
const WM_APP_PROGRESS: u32 = WM_APP + 2;
const WM_APP_DONE: u32 = WM_APP + 3;

#[derive(Deserialize)]
struct CefIndex {
    windows64: PlatformIndex,
}

#[derive(Deserialize)]
struct PlatformIndex {
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize)]
struct VersionEntry {
    cef_version: String,
    files: Vec<FileEntry>,
}

#[derive(Deserialize)]
struct FileEntry {
    #[serde(rename = "type")]
    file_type: String,
    name: String,
    sha1: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn runtime_dir(exe_dir: &Path) -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA")
        && !local.trim().is_empty()
    {
        return PathBuf::from(local)
            .join("Harbor")
            .join("runtime")
            .join(CEF_VERSION);
    }
    exe_dir.join("cef_runtime").join(CEF_VERSION)
}

fn is_runtime_ready(dir: &Path) -> bool {
    dir.join("libcef.dll").is_file()
        && dir.join("icudtl.dat").is_file()
        && dir.join("harbor.exe").is_file()
}

fn copy_if_newer(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_file() {
        return Ok(());
    }
    let copy_needed = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(sm), Ok(dm)) => sm.len() != dm.len() || sm.modified().ok() != dm.modified().ok(),
        _ => true,
    };
    if copy_needed {
        fs::copy(src, dst)
            .map_err(|e| format!("Failed copying {} to {}: {e}", src.display(), dst.display()))?;
    }
    Ok(())
}

fn sync_app_files(exe_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    let dll = exe_dir.join("harbor.dll");
    if !dll.is_file() {
        return Err(format!(
            "harbor.dll was not found next to {}. Extract all files from the Harbor release archive.",
            exe_dir.display()
        ));
    }
    copy_if_newer(&dll, &dest_dir.join("harbor.dll"))?;
    copy_if_newer(&exe_dir.join("harbor.ico"), &dest_dir.join("harbor.ico"))?;
    Ok(())
}

pub fn maybe_bootstrap_and_launch() -> Result<bool, Box<dyn std::error::Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or("Could not determine executable directory")?;

    // If libcef.dll is already in exe_dir alongside a non-launcher setup, fall through.
    if exe_dir.join("libcef.dll").is_file() && exe_dir.join("icudtl.dat").is_file() {
        return Ok(false);
    }

    let target_dir = runtime_dir(exe_dir);
    if !is_runtime_ready(&target_dir) {
        download_with_modal(&target_dir)?;
    }
    sync_app_files(exe_dir, &target_dir)?;

    let browser_exe = target_dir.join("harbor.exe");
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    Command::new(&browser_exe)
        .args(args)
        .current_dir(&target_dir)
        .spawn()
        .map_err(|e| format!("Failed launching {}: {e}", browser_exe.display()))?;
    Ok(true)
}

struct ModalState {
    label: HWND,
    progress: HWND,
    cancel_flag: Arc<AtomicBool>,
    result: Arc<std::sync::Mutex<Option<Result<(), String>>>>,
    done: bool,
}

unsafe extern "system" fn modal_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWLP_USERDATA, GetWindowLongPtrW, SetWindowLongPtrW,
    };
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ModalState;
    match msg {
        WM_APP_STATUS => {
            if !state_ptr.is_null() {
                let text = unsafe { Box::from_raw(lparam as *mut Vec<u16>) };
                let state = unsafe { &*state_ptr };
                unsafe { SetWindowTextW(state.label, text.as_ptr()) };
            }
            0
        }
        WM_APP_PROGRESS => {
            if !state_ptr.is_null() {
                let state = unsafe { &*state_ptr };
                unsafe { SendMessageW(state.progress, PBM_SETPOS, wparam, 0) };
            }
            0
        }
        WM_APP_DONE => {
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                state.done = true;
            }
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            if id == IDCANCEL && !state_ptr.is_null() {
                let state = unsafe { &*state_ptr };
                state.cancel_flag.store(true, Ordering::Relaxed);
                let cancelling = wide("Cancelling...");
                unsafe { SetWindowTextW(state.label, cancelling.as_ptr()) };
            }
            0
        }
        WM_CLOSE => {
            if !state_ptr.is_null() {
                let state = unsafe { &*state_ptr };
                state.cancel_flag.store(true, Ordering::Relaxed);
            }
            0
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state_ptr));
                }
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn post_status(hwnd_raw: usize, msg: &str) {
    let boxed = Box::new(wide(msg));
    let ptr = Box::into_raw(boxed);
    unsafe { PostMessageW(hwnd_raw as HWND, WM_APP_STATUS, 0, ptr as LPARAM) };
}

fn post_progress(hwnd_raw: usize, pct: usize) {
    unsafe { PostMessageW(hwnd_raw as HWND, WM_APP_PROGRESS, pct.min(100), 0) };
}

fn download_with_modal(target_dir: &Path) -> Result<(), String> {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&icc);
    }

    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class_name = wide("HarborCefBootstrapModal");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(modal_proc),
        hInstance: hinstance,
        lpszClassName: class_name.as_ptr(),
        hbrBackground: (windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW + 1) as _,
        ..Default::default()
    };
    unsafe { RegisterClassW(&wc) };

    let win_w = 460;
    let win_h = 165;
    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let x = ((screen_w - win_w) / 2).max(0);
    let y = ((screen_h - win_h) / 2).max(0);

    let title = wide("Harbor Browser Setup");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            win_w,
            win_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("Failed creating setup progress window.".into());
    }

    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let static_cls = wide("STATIC");
    let initial_txt = wide("Preparing to download Harbor Browser runtime (CEF 156.0.31)...");
    let label = unsafe {
        CreateWindowExW(
            0,
            static_cls.as_ptr(),
            initial_txt.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            405,
            24,
            hwnd,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    unsafe { SendMessageW(label, WM_SETFONT, font as WPARAM, 1) };

    let progress = unsafe {
        CreateWindowExW(
            0,
            PROGRESS_CLASSW,
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | PBS_SMOOTH,
            20,
            52,
            405,
            22,
            hwnd,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    unsafe { SendMessageW(progress, PBM_SETRANGE32, 0, 100) };

    let btn_cls = wide("BUTTON");
    let cancel_txt = wide("Cancel");
    let cancel_btn = unsafe {
        CreateWindowExW(
            0,
            btn_cls.as_ptr(),
            cancel_txt.as_ptr(),
            WS_CHILD | WS_VISIBLE | (BS_PUSHBUTTON as u32),
            345,
            88,
            80,
            26,
            hwnd,
            IDCANCEL as _,
            hinstance,
            std::ptr::null(),
        )
    };
    unsafe { SendMessageW(cancel_btn, WM_SETFONT, font as WPARAM, 1) };

    let cancel_flag = Arc::new(AtomicBool::new(false));
    let result = Arc::new(std::sync::Mutex::new(None));
    let state = Box::new(ModalState {
        label,
        progress,
        cancel_flag: cancel_flag.clone(),
        result: result.clone(),
        done: false,
    });
    let state_ptr = Box::into_raw(state);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            state_ptr as isize,
        );
        ShowWindow(hwnd, SW_SHOW);
    }

    let hwnd_raw = hwnd as usize;
    let target = target_dir.to_path_buf();
    let res_clone = result.clone();
    std::thread::spawn(move || {
        let outcome = run_download_and_extract(hwnd_raw, &target, &cancel_flag);
        *res_clone.lock().unwrap() = Some(outcome);
        unsafe { PostMessageW(hwnd_raw as HWND, WM_APP_DONE, 0, 0) };
    });

    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let outcome = result
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| Err("Setup was interrupted.".into()));
    if let Err(ref err) = outcome {
        let err_wide = wide(err);
        let title_wide = wide("Harbor Browser Setup");
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                err_wide.as_ptr(),
                title_wide.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        let _ = IDOK;
    }
    outcome
}

fn resolve_cef_archive(client: &reqwest::Client) -> (String, String) {
    let index_url = format!("{DEFAULT_CDN}/index.json");
    if let Ok(resp) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
        .and_then(|rt| {
            rt.block_on(async {
                let res = client.get(&index_url).send().await.ok()?;
                res.json::<CefIndex>().await.ok()
            })
        })
        .ok_or(())
    {
        let prefix = format!("{CEF_VERSION}+");
        for ver in resp.windows64.versions {
            if ver.cef_version.starts_with(&prefix)
                && let Some(file) = ver.files.into_iter().find(|f| f.file_type == "minimal")
            {
                return (file.name, file.sha1);
            }
        }
    }
    (FALLBACK_ARCHIVE.to_string(), FALLBACK_SHA1.to_string())
}

fn run_download_and_extract(
    hwnd_raw: usize,
    target_dir: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    fs::create_dir_all(target_dir).map_err(|e| format!("Failed creating runtime dir: {e}"))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("Failed initializing HTTP client: {e}"))?;

    post_status(hwnd_raw, "Resolving CEF 156.0.31 package...");
    let (archive_name, expected_sha1) = resolve_cef_archive(&client);
    let archive_url = format!("{DEFAULT_CDN}/{archive_name}");
    let tmp_archive = target_dir.join(format!("{archive_name}.part"));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed starting async runtime: {e}"))?;

    rt.block_on(async {
        let mut resp = client
            .get(&archive_url)
            .send()
            .await
            .map_err(|e| format!("Failed downloading CEF runtime: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("CEF download failed with HTTP {}.", resp.status()));
        }
        let total = resp.content_length().unwrap_or(0);
        let mut out = File::create(&tmp_archive)
            .map_err(|e| format!("Failed creating temporary archive: {e}"))?;
        let mut downloaded: u64 = 0;
        let mut last_pct: usize = 101;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("Download interrupted: {e}"))?
        {
            if cancel.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&tmp_archive);
                return Err("Download cancelled by user.".into());
            }
            out.write_all(&chunk)
                .map_err(|e| format!("Failed writing archive: {e}"))?;
            downloaded += chunk.len() as u64;
            if let Some(pct_u64) = downloaded.saturating_mul(85).checked_div(total) {
                let pct = pct_u64 as usize;
                if pct != last_pct {
                    last_pct = pct;
                    post_progress(hwnd_raw, pct);
                    let mb_done = downloaded as f64 / (1024.0 * 1024.0);
                    let mb_total = total as f64 / (1024.0 * 1024.0);
                    post_status(
                        hwnd_raw,
                        &format!(
                            "Downloading CEF {CEF_VERSION}... {mb_done:.1} / {mb_total:.1} MB"
                        ),
                    );
                }
            }
        }
        Ok::<(), String>(())
    })?;

    post_status(hwnd_raw, "Verifying CEF archive checksum...");
    post_progress(hwnd_raw, 88);
    let actual_sha1 = compute_sha1(&tmp_archive)?;
    if !actual_sha1.eq_ignore_ascii_case(&expected_sha1) {
        let _ = fs::remove_file(&tmp_archive);
        return Err("Downloaded CEF archive failed SHA-1 verification.".into());
    }

    post_status(hwnd_raw, "Extracting CEF runtime files...");
    post_progress(hwnd_raw, 92);
    extract_cef_tar_bz2(&tmp_archive, target_dir, cancel)?;
    let _ = fs::remove_file(&tmp_archive);

    let archive_meta = serde_json::json!({
        "type": "minimal",
        "name": archive_name,
        "sha1": expected_sha1,
    });
    fs::write(
        target_dir.join("archive.json"),
        serde_json::to_vec_pretty(&archive_meta).unwrap_or_default(),
    )
    .map_err(|e| format!("Failed writing archive.json: {e}"))?;

    post_progress(hwnd_raw, 100);
    post_status(hwnd_raw, "Ready. Launching Harbor Browser...");
    Ok(())
}

fn compute_sha1(path: &Path) -> Result<String, String> {
    let mut reader = BufReader::new(File::open(path).map_err(|e| format!("Open error: {e}"))?);
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Read error: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.digest().to_string())
}

fn extract_cef_tar_bz2(
    archive_path: &Path,
    dest_dir: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let file = File::open(archive_path).map_err(|e| format!("Failed opening archive: {e}"))?;
    let bz = BzDecoder::new(BufReader::new(file));
    let mut tar = Archive::new(bz);
    let entries = tar
        .entries()
        .map_err(|e| format!("Failed reading tar entries: {e}"))?;

    let mut seen_dirs: HashMap<PathBuf, ()> = HashMap::new();
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err("Extraction cancelled by user.".into());
        }
        let mut entry = entry.map_err(|e| format!("Corrupt tar entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let raw_path = entry
            .path()
            .map_err(|e| format!("Invalid tar entry path: {e}"))?
            .into_owned();
        let mut comps = raw_path.components();
        // Strip top-level archive folder (cef_binary_...).
        let _root = comps.next();
        let rel: PathBuf = comps.collect();
        let mut rel_comps = rel.components();
        let Some(Component::Normal(first)) = rel_comps.next() else {
            continue;
        };
        let first_str = first.to_string_lossy();
        let sub_rel: PathBuf = if first_str == "Release" || first_str == "Resources" {
            rel_comps.collect()
        } else if first_str == "CREDITS.html" {
            rel.clone()
        } else {
            continue;
        };

        // Guard against path traversal.
        if sub_rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            continue;
        }

        let out_name = if sub_rel == Path::new("bootstrap.exe") {
            PathBuf::from("harbor.exe")
        } else {
            sub_rel
        };
        let out_path = dest_dir.join(&out_name);
        if let Some(parent) = out_path.parent()
            && !seen_dirs.contains_key(parent)
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed creating {}: {e}", parent.display()))?;
            seen_dirs.insert(parent.to_path_buf(), ());
        }
        let mut out_file = File::create(&out_path)
            .map_err(|e| format!("Failed writing {}: {e}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("Failed extracting {}: {e}", out_path.display()))?;
    }
    Ok(())
}
