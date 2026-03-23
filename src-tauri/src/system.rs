use crate::AppRuntime;
use sysinfo::{Pid, System};

#[tauri::command]
pub fn system_restart_app(app_handle: tauri::AppHandle<AppRuntime>) {
    log::info!("restarting app via updater toast");
    app_handle.restart();
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
