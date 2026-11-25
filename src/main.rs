#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod cef_bootstrap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    if cef_bootstrap::maybe_bootstrap_and_launch()? {
        return Ok(());
    }
    harbor::run_from_executable()
}
