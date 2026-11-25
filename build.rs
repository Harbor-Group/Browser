use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    // I check PATH for rc.exe before searching Windows Kits.
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return Some(PathBuf::from("rc.exe"));
    }

    let kits_root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if kits_root.exists()
        && let Ok(entries) = std::fs::read_dir(kits_root)
    {
        let mut versions: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("x64").join("rc.exe").exists())
            .collect();
        versions.sort();
        if let Some(newest) = versions.last() {
            return Some(newest.join("x64").join("rc.exe"));
        }
    }

    None
}

fn main() {
    println!("cargo:rerun-if-changed=harbor.ico");
    println!("cargo:rerun-if-changed=harbor.rc");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let rc_path = manifest_dir.join("harbor.rc");
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let res_path = out_dir.join("harbor.res");

        if let Some(rc_exe) = find_rc_exe() {
            let status = Command::new(&rc_exe)
                .current_dir(&manifest_dir)
                .arg(format!("/fo{}", res_path.display()))
                .arg(&rc_path)
                .status();

            match status {
                Ok(s) if s.success() => {
                    println!("cargo:rustc-link-arg={}", res_path.display());
                }
                other => {
                    eprintln!("cargo:warning=rc.exe compilation failed: {:?}", other);
                }
            }
        } else {
            eprintln!("cargo:warning=rc.exe not found; skipping Windows icon resource compilation");
        }
    }
}
