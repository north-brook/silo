mod claude;
mod codex;
mod config;
mod gcloud;
mod gh;
mod logging;
mod projects;
mod river_names;
mod system;
mod workspaces;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (logging_plugin, session_log) = logging::build_plugin();

    tauri::Builder::default()
        .plugin(logging_plugin)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(move |_app| {
            if let Err(error) = config::initialize_on_start() {
                log::error!("failed to initialize silo config: {error}");
            }

            if let Some(session_log) = &session_log {
                log::info!(
                    "session logging initialized at {}",
                    session_log.path.display()
                );
            } else {
                log::warn!("session file logging is unavailable; using stdout only");
            }

            log::info!("silo backend startup complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            claude::claude_authenticate,
            claude::claude_configured,
            codex::codex_authenticate,
            codex::codex_configured,
            gh::gh_installed,
            gh::gh_configured,
            gh::gh_username,
            gh::gh_project_branches,
            gcloud::gcloud_authenticate,
            gcloud::gcloud_configure,
            gcloud::gcloud_installed,
            gcloud::gcloud_configured,
            gcloud::gcloud_accounts,
            gcloud::gcloud_projects,
            projects::projects_list_projects,
            projects::projects_add_project,
            projects::projects_update_project,
            projects::projects_reorder_projects,
            workspaces::workspaces_list_workspaces,
            workspaces::workspaces_create_workspace,
            workspaces::workspaces_start_workspace,
            workspaces::workspaces_stop_workspace,
            workspaces::workspaces_get_workspace,
            workspaces::workspaces_delete_workspace,
            workspaces::workspaces_update_workspace_branch,
            workspaces::workspaces_update_workspace_target_branch,
            system::system_memory_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
