mod claude;
mod codex;
mod config;
mod gcloud;
mod git;
mod logging;
mod projects;
mod river_names;
mod system;
mod templates;
mod terminal;
mod workspaces;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (logging_plugin, session_log) = logging::build_plugin();

    tauri::Builder::default()
        .manage(terminal::TerminalManager::default())
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
            git::git_authenticate,
            git::git_installed,
            git::git_configured,
            git::git_username,
            git::git_project_branches,
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
            templates::templates_list_templates,
            templates::templates_create_template,
            templates::templates_edit_template,
            templates::templates_save_template,
            templates::templates_delete_template,
            workspaces::workspaces_list_workspaces,
            workspaces::workspaces_create_workspace,
            workspaces::workspaces_start_workspace,
            workspaces::workspaces_stop_workspace,
            workspaces::workspaces_get_workspace,
            workspaces::workspaces_delete_workspace,
            workspaces::workspaces_update_workspace_branch,
            workspaces::workspaces_update_workspace_target_branch,
            terminal::terminal_create_terminal,
            terminal::terminal_list_terminals,
            terminal::terminal_attach_terminal,
            terminal::terminal_run_terminal,
            terminal::terminal_detach_terminal,
            terminal::terminal_kill_terminal,
            terminal::terminal_write_terminal,
            terminal::terminal_resize_terminal,
            system::system_memory_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
