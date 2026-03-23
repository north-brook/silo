use crate::AppRuntime;
use sysinfo::{Pid, System};
#[cfg(target_os = "macos")]
use tauri::Manager;

#[tauri::command]
pub fn system_restart_app(app_handle: tauri::AppHandle<AppRuntime>) {
    #[cfg(target_os = "macos")]
    {
        // Avoid the CEF shutdown path here. On update relaunch it can segfault
        // during exit, even though the new bundle is already installed.
        log::info!("restarting app via updater toast using macOS hard restart workaround");
        app_handle.cleanup_before_exit();
        tauri::process::restart(&app_handle.env());
    }

    #[cfg(not(target_os = "macos"))]
    {
        log::info!("restarting app via updater toast");
        app_handle.restart();
    }
}

#[tauri::command]
pub fn system_memory_usage() -> f64 {
    log::trace!("reading process memory usage");
    let pid = Pid::from_u32(std::process::id());
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);

    let bytes = sys.process(pid).map(|p| p.memory()).unwrap_or(0);

    let mb = (bytes as f64) / 1_048_576.0;
    let rounded = (mb * 10.0).round() / 10.0;
    log::trace!("process memory usage is {rounded:.1} MB");
    rounded
}
