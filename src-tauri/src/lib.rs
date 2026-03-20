mod bootstrap;
mod browser;
mod browser_loopback;
mod claude;
mod codex;
mod config;
mod files;
mod gcloud;
mod git;
mod logging;
mod projects;
mod prompts;
mod remote;
mod river_names;
mod router;
mod state;
mod state_paths;
mod system;
mod templates;
mod terminal;
mod tls;
mod workspaces;

use std::env;
use tauri::{AppHandle, Cef, Emitter, Manager};

pub type AppRuntime = Cef;
const MAIN_SHELL_LABEL: &str = "main";
pub(crate) const WORKSPACE_STATE_EVENT_NAME: &str = "workspace://state";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceStateEvent {
    workspace: String,
    cleared_active_session: bool,
    removed_session_attachment_id: Option<String>,
    removed_session_kind: Option<String>,
}

const MENU_ID_NEW_WORKSPACE: &str = "new_workspace";
const MENU_ID_OPEN_PROJECT: &str = "open_project";
const MENU_ID_NEW_TAB: &str = "new_tab";
const MENU_ID_CLOSE_TAB: &str = "close_tab";
const MENU_ID_GO_BACK_BROWSER: &str = "go_back_browser";
const MENU_ID_GO_FORWARD_BROWSER: &str = "go_forward_browser";
const MENU_ID_REFRESH_BROWSER: &str = "refresh_browser";
const MENU_ID_TOGGLE_BROWSER_DEVTOOLS: &str = "toggle_browser_devtools";
const MENU_ID_PREVIOUS_TAB: &str = "previous_tab";
const MENU_ID_NEXT_TAB: &str = "next_tab";
const MENU_ID_TOGGLE_PROJECTS_BAR: &str = "toggle_projects_bar";
const MENU_ID_TOGGLE_GIT_BAR: &str = "toggle_git_bar";
const MENU_ID_OPEN_GIT_DIFF: &str = "open_git_diff";
const MENU_ID_OPEN_GIT_FILES: &str = "open_git_files";
const MENU_ID_OPEN_GIT_CHECKS: &str = "open_git_checks";
const MENU_ID_GIT_CREATE_OR_PUSH_PR: &str = "git_create_or_push_pr";
const MENU_ID_GIT_MERGE_PR: &str = "git_merge_pr";
const MENU_ID_CLOSE_WINDOW: &str = "close_window";
const MENU_ID_QUIT_APP: &str = "quit_app";
const MENU_ID_JUMP_TO_WORKSPACE_PREFIX: &str = "jump_to_workspace_";

const SHORTCUT_EVENT_NEW_WORKSPACE: &str = "silo://new-workspace";
const SHORTCUT_EVENT_OPEN_PROJECT: &str = "silo://open-project";
const SHORTCUT_EVENT_NEW_TAB: &str = "silo://new-tab";
const SHORTCUT_EVENT_CLOSE_TAB: &str = "silo://close-tab";
const SHORTCUT_EVENT_GO_BACK_BROWSER: &str = "silo://go-back-browser";
const SHORTCUT_EVENT_GO_FORWARD_BROWSER: &str = "silo://go-forward-browser";
const SHORTCUT_EVENT_REFRESH_BROWSER: &str = "silo://refresh-browser";
const SHORTCUT_EVENT_TOGGLE_BROWSER_DEVTOOLS: &str = "silo://toggle-browser-devtools";
const SHORTCUT_EVENT_PREVIOUS_TAB: &str = "silo://previous-tab";
const SHORTCUT_EVENT_NEXT_TAB: &str = "silo://next-tab";
const SHORTCUT_EVENT_TOGGLE_PROJECTS_BAR: &str = "silo://toggle-projects-bar";
const SHORTCUT_EVENT_TOGGLE_GIT_BAR: &str = "silo://toggle-git-bar";
const SHORTCUT_EVENT_OPEN_GIT_DIFF: &str = "silo://open-git-diff";
const SHORTCUT_EVENT_OPEN_GIT_FILES: &str = "silo://open-git-files";
const SHORTCUT_EVENT_OPEN_GIT_CHECKS: &str = "silo://open-git-checks";
const SHORTCUT_EVENT_GIT_CREATE_OR_PUSH_PR: &str = "silo://git-create-or-push-pr";
const SHORTCUT_EVENT_GIT_MERGE_PR: &str = "silo://git-merge-pr";
const SHORTCUT_EVENT_JUMP_TO_WORKSPACE: &str = "silo://jump-to-workspace";

fn emit_shortcut_event(app_handle: &AppHandle<AppRuntime>, event: &str) {
    let _ = app_handle.emit(event, ());
}

pub(crate) fn emit_workspace_state_changed(
    app_handle: &AppHandle<AppRuntime>,
    workspace: &str,
    removed_session: Option<(&str, &str)>,
    cleared_active_session: bool,
) {
    let (removed_session_kind, removed_session_attachment_id) = removed_session
        .map(|(kind, attachment_id)| (Some(kind.to_string()), Some(attachment_id.to_string())))
        .unwrap_or((None, None));
    let _ = app_handle.emit(
        WORKSPACE_STATE_EVENT_NAME,
        WorkspaceStateEvent {
            workspace: workspace.to_string(),
            cleared_active_session,
            removed_session_attachment_id,
            removed_session_kind,
        },
    );
}

fn emit_workspace_jump_event(app_handle: &AppHandle<AppRuntime>, index: u8) {
    let _ = app_handle.emit(SHORTCUT_EVENT_JUMP_TO_WORKSPACE, index);
}

fn shortcut_targets_main_shell(menu_id: &str) -> bool {
    matches!(
        menu_id,
        MENU_ID_NEW_WORKSPACE
            | MENU_ID_OPEN_PROJECT
            | MENU_ID_NEW_TAB
            | MENU_ID_CLOSE_TAB
            | MENU_ID_PREVIOUS_TAB
            | MENU_ID_NEXT_TAB
            | MENU_ID_TOGGLE_PROJECTS_BAR
            | MENU_ID_TOGGLE_GIT_BAR
            | MENU_ID_OPEN_GIT_DIFF
            | MENU_ID_OPEN_GIT_FILES
            | MENU_ID_OPEN_GIT_CHECKS
            | MENU_ID_GIT_CREATE_OR_PUSH_PR
            | MENU_ID_GIT_MERGE_PR
    ) || menu_id.starts_with(MENU_ID_JUMP_TO_WORKSPACE_PREFIX)
}

fn restore_main_shell_focus(app_handle: &AppHandle<AppRuntime>, menu_id: &str) {
    if let Some(window) = app_handle.get_window(MAIN_SHELL_LABEL) {
        match window.set_focus() {
            Ok(()) => {
                log::info!("requested main window focus for native menu event menu_id={menu_id}");
            }
            Err(error) => {
                log::warn!(
                    "failed to focus main window for native menu event menu_id={menu_id}: {error}"
                );
            }
        }
    } else {
        log::warn!("main window missing for native menu event menu_id={menu_id}");
    }

    if let Some(webview) = app_handle.get_webview(MAIN_SHELL_LABEL) {
        match webview.set_focus() {
            Ok(()) => {
                log::info!("requested main webview focus for native menu event menu_id={menu_id}");
            }
            Err(error) => {
                log::warn!(
                    "failed to focus main webview for native menu event menu_id={menu_id}: {error}"
                );
            }
        }
    } else {
        log::warn!("main webview missing for native menu event menu_id={menu_id}");
    }
}

fn handle_shortcut_menu_event(app_handle: &AppHandle<AppRuntime>, menu_id: &str) -> bool {
    // Canonical shortcut routing lives in the native menu.
    // Add new app-level shortcuts here first, then listen for the emitted
    // `silo://...` event in the frontend. Only touch the CEF/AppKit shims when
    // Chromium prevents a registered menu shortcut from reaching this path.
    log::info!("received native menu event menu_id={menu_id}");
    if shortcut_targets_main_shell(menu_id) {
        restore_main_shell_focus(app_handle, menu_id);
    }
    match menu_id {
        MENU_ID_NEW_WORKSPACE => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEW_WORKSPACE),
        MENU_ID_OPEN_PROJECT => emit_shortcut_event(app_handle, SHORTCUT_EVENT_OPEN_PROJECT),
        MENU_ID_NEW_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEW_TAB),
        MENU_ID_CLOSE_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_CLOSE_TAB),
        MENU_ID_GO_BACK_BROWSER => emit_shortcut_event(app_handle, SHORTCUT_EVENT_GO_BACK_BROWSER),
        MENU_ID_GO_FORWARD_BROWSER => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_GO_FORWARD_BROWSER)
        }
        MENU_ID_REFRESH_BROWSER => emit_shortcut_event(app_handle, SHORTCUT_EVENT_REFRESH_BROWSER),
        MENU_ID_TOGGLE_BROWSER_DEVTOOLS => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_TOGGLE_BROWSER_DEVTOOLS)
        }
        MENU_ID_PREVIOUS_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_PREVIOUS_TAB),
        MENU_ID_NEXT_TAB => emit_shortcut_event(app_handle, SHORTCUT_EVENT_NEXT_TAB),
        MENU_ID_TOGGLE_PROJECTS_BAR => {
            emit_shortcut_event(app_handle, SHORTCUT_EVENT_TOGGLE_PROJECTS_BAR)
        }
        MENU_ID_TOGGLE_GIT_BAR => emit_shortcut_event(app_handle, SHORTCUT_EVENT_TOGGLE_GIT_BAR),
        MENU_ID_OPEN_GIT_DIFF => emit_shortcut_event(app_handle, SHORTCUT_EVENT_OPEN_GIT_DIFF),
        MENU_ID_OPEN_GIT_FILES => emit_shortcut_event(app_handle, SHORTCUT_EVENT_OPEN_GIT_FILES),
        MENU_ID_OPEN_GIT_CHECKS => emit_shortcut_event(app_handle, SHORTCUT_EVENT_OPEN_GIT_CHECKS),
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
            if !(0..=9).contains(&index) {
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
    let cef_command_line_args = cef_command_line_args();
    let loopback_router = router::RouterManager::default();
    let browser_manager = browser::BrowserManager::new(loopback_router.clone());
    let browser_loopback_manager = browser_loopback::BrowserLoopbackManager::new(loopback_router);
    let browser_loopback_resolver = browser_loopback_manager.clone();
    let _ = tauri_runtime_cef::set_loopback_request_resolver(std::sync::Arc::new(
        move |webview_label, original_url| {
            browser_loopback_resolver.rewrite_loopback_url(webview_label, original_url)
        },
    ));

    tauri::Builder::<AppRuntime>::new()
        .command_line_args(cef_command_line_args)
        .manage(browser_manager)
        .manage(browser_loopback_manager)
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

            templates::remove_legacy_template_state_file();

            if let Some(session_log) = &session_log {
                log::info!(
                    "session logging initialized at {}",
                    session_log.path.display()
                );
                if let Some(trace_id) = env::var_os("SILO_TRACE_ID") {
                    log::info!(
                        "trace logging active trace_id={} directory={}",
                        trace_id.to_string_lossy(),
                        session_log.directory.display()
                    );
                }
            } else {
                log::warn!("session file logging is unavailable; using stdout only");
            }

            let workspace_state = app
                .state::<state::WorkspaceMetadataManager>()
                .inner()
                .clone();
            bootstrap::initialize_workspace_metadata_manager(workspace_state.clone());
            templates::resume_running_template_operations(workspace_state);

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
                    "Refresh",
                    true,
                    Some("CmdOrCtrl+R"),
                )?;
                let toggle_browser_devtools = MenuItem::with_id(
                    handle,
                    MENU_ID_TOGGLE_BROWSER_DEVTOOLS,
                    "Toggle DevTools",
                    true,
                    Some("CmdOrCtrl+Shift+I"),
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
                    Some("CmdOrCtrl+Alt+B"),
                )?;
                let open_git_diff = MenuItem::with_id(
                    handle,
                    MENU_ID_OPEN_GIT_DIFF,
                    "Show Diff",
                    true,
                    Some("CmdOrCtrl+Shift+D"),
                )?;
                let open_git_files = MenuItem::with_id(
                    handle,
                    MENU_ID_OPEN_GIT_FILES,
                    "Show Files",
                    true,
                    Some("CmdOrCtrl+Shift+E"),
                )?;
                let open_git_checks = MenuItem::with_id(
                    handle,
                    MENU_ID_OPEN_GIT_CHECKS,
                    "Show Checks",
                    true,
                    Some("CmdOrCtrl+Shift+C"),
                )?;
                let git_create_or_push_pr = MenuItem::with_id(
                    handle,
                    MENU_ID_GIT_CREATE_OR_PUSH_PR,
                    "Create or Push PR",
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
                let quit_app_item = MenuItem::with_id(
                    handle,
                    MENU_ID_QUIT_APP,
                    "Quit Silo",
                    true,
                    Some("CmdOrCtrl+Q"),
                )?;

                let dashboard_jump = MenuItem::with_id(
                    handle,
                    format!("{MENU_ID_JUMP_TO_WORKSPACE_PREFIX}0"),
                    "Dashboard",
                    true,
                    Some("CmdOrCtrl+0"),
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
                    .map(|item| item as &dyn IsMenuItem<AppRuntime>)
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
                    .item(&quit_app_item)
                    .build()?;

                let file_submenu = SubmenuBuilder::new(handle, "File")
                    .items(&[&new_workspace, &open_project, &new_tab, &close_tab])
                    .build()?;

                let jump_to_workspace_submenu = SubmenuBuilder::new(handle, "Jump to Workspace")
                    .items(&workspace_jump_refs)
                    .build()?;

                let browser_submenu = SubmenuBuilder::new(handle, "Browser")
                    .items(&[
                        &go_back_browser,
                        &go_forward_browser,
                        &refresh_browser,
                        &toggle_browser_devtools,
                    ])
                    .build()?;

                let navigate_submenu = SubmenuBuilder::new(handle, "Navigate")
                    .item(&dashboard_jump)
                    .separator()
                    .items(&[&previous_tab, &next_tab])
                    .separator()
                    .item(&jump_to_workspace_submenu)
                    .build()?;

                let view_submenu = SubmenuBuilder::new(handle, "View")
                    .items(&[&toggle_projects_bar, &toggle_git_bar])
                    .build()?;

                let git_submenu = SubmenuBuilder::new(handle, "Git")
                    .items(&[
                        &open_git_diff,
                        &open_git_files,
                        &open_git_checks,
                        &git_create_or_push_pr,
                        &git_merge_pr,
                    ])
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
                    .item(&browser_submenu)
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
                } else if event.id() == MENU_ID_QUIT_APP {
                    quit_application(app_handle);
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
            templates::templates_get_state,
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
            workspaces::workspaces_set_active_session,
            workspaces::workspaces_submit_prompt,
            workspaces::workspaces_delete_workspace,
            browser::browser_create_tab,
            browser::browser_mount_tab,
            browser::browser_resize_tab,
            browser::browser_detach_tab,
            browser::browser_kill_tab,
            browser::browser_go_to,
            browser::browser_report_page_state,
            browser::browser_go_back,
            browser::browser_go_forward,
            browser::browser_refresh_page,
            browser::browser_open_devtools,
            browser::browser_toggle_devtools,
            terminal::terminal_create_terminal,
            terminal::terminal_create_assistant,
            terminal::terminal_list_terminals,
            terminal::terminal_attach_terminal,
            terminal::terminal_run_terminal,
            terminal::terminal_detach_terminal,
            terminal::terminal_kill_terminal,
            terminal::terminal_read_terminal,
            terminal::terminal_write_terminal,
            terminal::terminal_finish_attach,
            terminal::terminal_resize_terminal,
            files::files_list_tree,
            files::files_read,
            files::files_save,
            files::files_set_watched_paths,
            files::files_get_watched_state,
            files::files_open_session,
            files::files_close_session,
            system::system_memory_usage
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(handle_run_event);
}

fn handle_run_event(app_handle: &tauri::AppHandle<AppRuntime>, event: tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
        if code != Some(tauri::RESTART_EXIT_CODE) {
            log::info!(
                "routing macOS exit request through hard exit workaround for CEF code={code:?}"
            );
            api.prevent_exit();
            quit_application(app_handle);
        }
    }
}

fn cef_command_line_args() -> Vec<(String, Option<String>)> {
    let mut args = Vec::new();

    if cfg!(debug_assertions) && std::env::var("SILO_CEF_USE_MOCK_KEYCHAIN").as_deref() != Ok("0") {
        // CEF workaround - reevaluate when CEF is stable.
        args.push(("--use-mock-keychain".to_string(), None));
    }

    if let Ok(remote_debugging_port) = std::env::var("SILO_CEF_REMOTE_DEBUGGING_PORT") {
        let remote_debugging_port = remote_debugging_port.trim();
        if !remote_debugging_port.is_empty() {
            args.push((
                "remote-debugging-port".to_string(),
                Some(remote_debugging_port.to_string()),
            ));
        }
    }

    args
}

fn quit_application(app_handle: &tauri::AppHandle<AppRuntime>) {
    #[cfg(target_os = "macos")]
    {
        // CEF workaround - reevaluate when CEF is stable.
        log::info!("quitting Silo via hard exit workaround for CEF on macOS");
        app_handle.cleanup_before_exit();
        std::process::exit(0);
    }

    #[cfg(not(target_os = "macos"))]
    {
        app_handle.exit(0);
    }
}
