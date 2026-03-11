mod claude;
mod codex;
mod config;
mod gcloud;
mod gh;
mod projects;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = config::initialize_on_start() {
        eprintln!("failed to initialize silo config: {error}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            claude::claude_authenticate,
            claude::claude_configured,
            codex::codex_authenticate,
            codex::codex_configured,
            gh::gh_installed,
            gh::gh_configured,
            gh::gh_username,
            gcloud::gcloud_authenticate,
            gcloud::gcloud_configure,
            gcloud::gcloud_installed,
            gcloud::gcloud_configured,
            gcloud::gcloud_accounts,
            gcloud::gcloud_projects,
            projects::projects_list_projects,
            projects::projects_add_project,
            projects::projects_update_project,
            projects::projects_reorder_projects
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
