mod browser;
mod claude;
mod codex;
mod config;
mod gcloud;
mod git;
mod logging;
mod projects;
mod prompts;
mod river_names;
mod router;
mod state;
mod system;
mod templates;
mod terminal;
mod tls;
mod workspaces;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use tauri::{Emitter, Manager};

    let (logging_plugin, session_log) = logging::build_plugin();

    tauri::Builder::default()
        .manage(browser::BrowserManager::default())
        .manage(state::WorkspaceMetadataManager::default())
        .manage(terminal::TerminalManager::default())
        .plugin(logging_plugin)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(move |app| {
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

            // Custom menu: replace default Cmd+W (Close Window) with Close Tab
            {
                use tauri::menu::*;
                let handle = app.handle();

                let close_tab = MenuItem::with_id(
                    handle,
                    "close_tab",
                    "Close Tab",
                    true,
                    Some("CmdOrCtrl+W"),
                )?;

                let close_window_item = MenuItem::with_id(
                    handle,
                    "close_window",
                    "Close Window",
                    true,
                    None::<&str>,
                )?;

                let app_submenu = SubmenuBuilder::new(handle, "Silo")
                    .about(None)
                    .separator()
                    .services()
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;

                let file_submenu = SubmenuBuilder::new(handle, "File")
                    .item(&close_tab)
                    .build()?;

                let edit_submenu = SubmenuBuilder::new(handle, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;

                let window_submenu = SubmenuBuilder::new(handle, "Window")
                    .minimize()
                    .separator()
                    .item(&close_window_item)
                    .build()?;

                let menu = MenuBuilder::new(handle)
                    .item(&app_submenu)
                    .item(&file_submenu)
                    .item(&edit_submenu)
                    .item(&window_submenu)
                    .build()?;

                app.set_menu(menu)?;
            }

            app.on_menu_event(|app_handle, event| {
                if event.id() == "close_tab" {
                    let _ = app_handle.emit("silo://close-tab", ());
                } else if event.id() == "close_window" {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.close();
                    }
                }
            });

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
            git::git_diff,
            git::git_update_branch,
            git::git_update_target_branch,
            git::git_pr_status,
            git::git_pr_observe,
            git::git_tree_dirty,
            git::git_push,
            git::git_create_pr,
            git::git_merge_pr,
            git::git_rerun_failed_checks,
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
            workspaces::workspaces_resume_workspace,
            workspaces::workspaces_stop_workspace,
            workspaces::workspaces_suspend_workspace,
            workspaces::workspaces_get_workspace,
            workspaces::workspaces_submit_prompt,
            workspaces::workspaces_delete_workspace,
            browser::browser_create_tab,
            browser::browser_mount_tab,
            browser::browser_resize_tab,
            browser::browser_unmount_tab,
            browser::browser_kill_tab,
            browser::browser_go_to,
            browser::browser_report_page_state,
            browser::browser_go_back,
            browser::browser_go_forward,
            browser::browser_refresh_page,
            browser::browser_open_devtools,
            terminal::terminal_create_terminal,
            terminal::terminal_create_assistant,
            terminal::terminal_list_terminals,
            terminal::terminal_attach_terminal,
            terminal::terminal_run_terminal,
            terminal::terminal_detach_terminal,
            terminal::terminal_kill_terminal,
            terminal::terminal_read_terminal,
            terminal::terminal_write_terminal,
            terminal::terminal_resize_terminal,
            system::system_memory_usage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
