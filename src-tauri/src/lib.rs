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

use tauri::{AppHandle, Emitter, Manager, Wry};

const MENU_ID_NEW_WORKSPACE: &str = "new_workspace";
const MENU_ID_OPEN_PROJECT: &str = "open_project";
const MENU_ID_NEW_TAB: &str = "new_tab";
const MENU_ID_CLOSE_TAB: &str = "close_tab";
const MENU_ID_GO_BACK_BROWSER: &str = "go_back_browser";
const MENU_ID_GO_FORWARD_BROWSER: &str = "go_forward_browser";
const MENU_ID_REFRESH_BROWSER: &str = "refresh_browser";
const MENU_ID_PREVIOUS_TAB: &str = "previous_tab";
const MENU_ID_NEXT_TAB: &str = "next_tab";
const MENU_ID_TOGGLE_PROJECTS_BAR: &str = "toggle_projects_bar";
const MENU_ID_TOGGLE_GIT_BAR: &str = "toggle_git_bar";
const MENU_ID_GIT_CREATE_OR_PUSH_PR: &str = "git_create_or_push_pr";
const MENU_ID_GIT_MERGE_PR: &str = "git_merge_pr";
const MENU_ID_CLOSE_WINDOW: &str = "close_window";
const MENU_ID_JUMP_TO_WORKSPACE_PREFIX: &str = "jump_to_workspace_";

const SHORTCUT_EVENT_NEW_WORKSPACE: &str = "silo://new-workspace";
const SHORTCUT_EVENT_OPEN_PROJECT: &str = "silo://open-project";
const SHORTCUT_EVENT_NEW_TAB: &str = "silo://new-tab";
const SHORTCUT_EVENT_CLOSE_TAB: &str = "silo://close-tab";
const SHORTCUT_EVENT_GO_BACK_BROWSER: &str = "silo://go-back-browser";
const SHORTCUT_EVENT_GO_FORWARD_BROWSER: &str = "silo://go-forward-browser";
const SHORTCUT_EVENT_REFRESH_BROWSER: &str = "silo://refresh-browser";
const SHORTCUT_EVENT_PREVIOUS_TAB: &str = "silo://previous-tab";
const SHORTCUT_EVENT_NEXT_TAB: &str = "silo://next-tab";
const SHORTCUT_EVENT_TOGGLE_PROJECTS_BAR: &str = "silo://toggle-projects-bar";
const SHORTCUT_EVENT_TOGGLE_GIT_BAR: &str = "silo://toggle-git-bar";
const SHORTCUT_EVENT_GIT_CREATE_OR_PUSH_PR: &str = "silo://git-create-or-push-pr";
const SHORTCUT_EVENT_GIT_MERGE_PR: &str = "silo://git-merge-pr";
const SHORTCUT_EVENT_JUMP_TO_WORKSPACE: &str = "silo://jump-to-workspace";

fn emit_shortcut_event(app_handle: &AppHandle<Wry>, event: &str) {
    let _ = app_handle.emit(event, ());
}

fn emit_workspace_jump_event(app_handle: &AppHandle<Wry>, index: u8) {
    let _ = app_handle.emit(SHORTCUT_EVENT_JUMP_TO_WORKSPACE, index);
}

fn handle_shortcut_menu_event(app_handle: &AppHandle<Wry>, menu_id: &str) -> bool {
    match menu_id {
        MENU_ID_NEW_WORKSPACE => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEW_WORKSPACE),
        MENU_ID_OPEN_PROJECT => emit_shortcut_event(app_handle, SHORTCUT_EVENT_OPEN_PROJECT),
        MENU_ID_NEW_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEW_TAB),
        MENU_ID_CLOSE_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_CLOSE_TAB),
        MENU_ID_GO_BACK_BROWSER => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_GO_BACK_BROWSER)
        }
        MENU_ID_GO_FORWARD_BROWSER => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_GO_FORWARD_BROWSER)
        }
        MENU_ID_REFRESH_BROWSER => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_REFRESH_BROWSER)
        }
        MENU_ID_PREVIOUS_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_PREVIOUS_TAB),
        MENU_ID_NEXT_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEXT_TAB),
        MENU_ID_TOGGLE_PROJECTS_BAR => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_TOGGLE_PROJECTS_BAR)
        }
        MENU_ID_TOGGLE_GIT_BAR => emit_shortcut_event(app_handle, SHORTCUT_EVENT_TOGGLE_GIT_BAR),
        MENU_ID_GIT_CREATE_OR_PUSH_PR => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_GIT_CREATE_OR_PUSH_PR)
        }
        MENU_ID_GIT_MERGE_PR => emit_shortcut_event(app_handle, SHORTCUT_EVENT_GIT_MERGE_PR),
        _ => {
            let Some(index) = menu_id.strip_prefix(MENU_ID_JUMP_TO_WORKSPACE_PREFIX) else {
                return false;
            };
            let Ok(index) = index.parse::<u8>() else {
                return false;
            };
            if !(1..=9).contains(&index) {
                return false;
            }
            emit_workspace_jump_event(app_handle, index);
        }
    }

    true
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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

            // Native accelerators ensure app shortcuts still work when focus moves to child webviews.
            {
                use tauri::menu::*;
                let handle = app.handle();

                let new_workspace = MenuItem::with_id(
                    handle,
                    MENU_ID_NEW_WORKSPACE,
                    "New Workspace",
                    true,
                    Some("CmdOrCtrl+N"),
                )?;
                let open_project = MenuItem::with_id(
                    handle,
                    MENU_ID_OPEN_PROJECT,
                    "Open Project…",
                    true,
                    Some("CmdOrCtrl+Shift+O"),
                )?;
                let new_tab = MenuItem::with_id(
                    handle,
                    MENU_ID_NEW_TAB,
                    "New Tab",
                    true,
                    Some("CmdOrCtrl+T"),
                )?;
                let close_tab = MenuItem::with_id(
                    handle,
                    MENU_ID_CLOSE_TAB,
                    "Close Tab",
                    true,
                    Some("CmdOrCtrl+W"),
                )?;
                let go_back_browser = MenuItem::with_id(
                    handle,
                    MENU_ID_GO_BACK_BROWSER,
                    "Back",
                    true,
                    Some("CmdOrCtrl+["),
                )?;
                let go_forward_browser = MenuItem::with_id(
                    handle,
                    MENU_ID_GO_FORWARD_BROWSER,
                    "Forward",
                    true,
                    Some("CmdOrCtrl+]"),
                )?;
                let refresh_browser = MenuItem::with_id(
                    handle,
                    MENU_ID_REFRESH_BROWSER,
                    "Refresh Page",
                    true,
                    Some("CmdOrCtrl+R"),
                )?;
                let previous_tab = MenuItem::with_id(
                    handle,
                    MENU_ID_PREVIOUS_TAB,
                    "Previous Tab",
                    true,
                    Some("CmdOrCtrl+Shift+["),
                )?;
                let next_tab = MenuItem::with_id(
                    handle,
                    MENU_ID_NEXT_TAB,
                    "Next Tab",
                    true,
                    Some("CmdOrCtrl+Shift+]"),
                )?;
                let toggle_projects_bar = MenuItem::with_id(
                    handle,
                    MENU_ID_TOGGLE_PROJECTS_BAR,
                    "Toggle Projects Bar",
                    true,
                    Some("CmdOrCtrl+B"),
                )?;
                let toggle_git_bar = MenuItem::with_id(
                    handle,
                    MENU_ID_TOGGLE_GIT_BAR,
                    "Toggle Git Bar",
                    true,
                    Some("CmdOrCtrl+Shift+B"),
                )?;
                let git_create_or_push_pr = MenuItem::with_id(
                    handle,
                    MENU_ID_GIT_CREATE_OR_PUSH_PR,
                    "Create Or Push PR",
                    true,
                    Some("CmdOrCtrl+Shift+P"),
                )?;
                let git_merge_pr = MenuItem::with_id(
                    handle,
                    MENU_ID_GIT_MERGE_PR,
                    "Merge PR",
                    true,
                    Some("CmdOrCtrl+Shift+M"),
                )?;
                let close_window_item = MenuItem::with_id(
                    handle,
                    MENU_ID_CLOSE_WINDOW,
                    "Close Window",
                    true,
                    None::<&str>,
                )?;

                let workspace_jump_items = (1..=9)
                    .map(|index| {
                        MenuItem::with_id(
                            handle,
                            format!("{MENU_ID_JUMP_TO_WORKSPACE_PREFIX}{index}"),
                            format!("Workspace {index}"),
                            true,
                            Some(format!("CmdOrCtrl+{index}")),
                        )
                    })
                    .collect::<tauri::Result<Vec<_>>>()?;
                let workspace_jump_refs = workspace_jump_items
                    .iter()
                    .map(|item| item as &dyn IsMenuItem<Wry>)
                    .collect::<Vec<_>>();

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
                    .items(&[&new_workspace, &open_project, &new_tab, &close_tab])
                    .build()?;

                let jump_to_workspace_submenu = SubmenuBuilder::new(handle, "Jump To Workspace")
                    .items(&workspace_jump_refs)
                    .build()?;

                let navigate_submenu = SubmenuBuilder::new(handle, "Navigate")
                    .items(&[
                        &go_back_browser,
                        &go_forward_browser,
                        &refresh_browser,
                        &previous_tab,
                        &next_tab,
                    ])
                    .separator()
                    .item(&jump_to_workspace_submenu)
                    .build()?;

                let view_submenu = SubmenuBuilder::new(handle, "View")
                    .items(&[&toggle_projects_bar, &toggle_git_bar])
                    .build()?;

                let git_submenu = SubmenuBuilder::new(handle, "Git")
                    .items(&[&git_create_or_push_pr, &git_merge_pr])
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
                    .item(&navigate_submenu)
                    .item(&view_submenu)
                    .item(&git_submenu)
                    .item(&edit_submenu)
                    .item(&window_submenu)
                    .build()?;

                app.set_menu(menu)?;
            }

            app.on_menu_event(|app_handle, event| {
                if handle_shortcut_menu_event(app_handle, event.id().as_ref()) {
                    return;
                }

                if event.id() == MENU_ID_CLOSE_WINDOW {
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
