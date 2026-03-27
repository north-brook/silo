use crate::agent_sessions;
use crate::bootstrap;
use crate::browser_file_server::{
    workspace_file_display_name_from_url, workspace_file_logical_url, BrowserFileServerManager,
};
use crate::browser_loopback::BrowserLoopbackManager;
use crate::files;
use crate::router::RouterManager;
use crate::state::WorkspaceMetadataManager;
use crate::terminal;
use crate::workspaces::{self, WorkspaceLookup, WorkspaceSession};
use crate::{emit_workspace_state_changed, AppRuntime};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use tauri::webview::{NewWindowResponse, PageLoadEvent, Url, WebviewBuilder};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Position, Rect, Size, State,
    Webview, WebviewUrl,
};

const BROWSER_KIND: &str = "browser";
const MAIN_WINDOW_LABEL: &str = "main";
const BROWSER_EVENT_NAME: &str = "browser://state";
const BROWSER_DEFAULT_URL: &str = "about:blank";
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserCreateResult {
    pub(crate) attachment_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserMountResult {
    pub(crate) attached: bool,
    pub(crate) session: WorkspaceSession,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserViewportResult {
    pub(crate) updated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserDetachResult {
    pub(crate) detached: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserKillResult {
    pub(crate) killed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserCommandResult {
    pub(crate) updated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BrowserMetadataResult {
    pub(crate) updated: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserViewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct BrowserWebviewState {
    resolved_url: String,
    viewport: BrowserViewport,
    visible: bool,
}

struct BrowserUrlTarget {
    logical_url: String,
    resolved_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BrowserStateEvent {
    workspace: String,
    attachment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    popup_attachment_id: Option<String>,
}

#[derive(Clone)]
pub struct BrowserManager {
    webviews: Arc<Mutex<HashMap<String, BrowserWebviewState>>>,
    sessions: Arc<Mutex<HashMap<String, WorkspaceSession>>>,
    file_server: BrowserFileServerManager,
    loopback_router: RouterManager,
}

#[tauri::command]
pub async fn browser_create_tab(
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    url: Option<String>,
) -> Result<BrowserCreateResult, String> {
    create_browser_tab(state.inner(), metadata.inner(), workspace, url).await
}

#[tauri::command]
pub async fn browser_open_workspace_file(
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    path: String,
) -> Result<BrowserCreateResult, String> {
    let lookup =
        workspaces::hydrate_workspace_lookup(files::branch_workspace_lookup(&workspace).await?)
            .await;
    let path = files::normalize_repo_relative_path(&path)?;
    if files::browser_renderable_content_type(&path).is_none() {
        return Err(format!(
            "workspace file does not support browser rendering: {path}"
        ));
    }

    let logical_url = workspace_file_logical_url(&workspace, &path)?;
    if let Some(existing) = find_browser_session_by_logical_url(
        state.inner(),
        &workspace,
        lookup.workspace.browsers(),
        &logical_url,
    )? {
        return Ok(BrowserCreateResult {
            attachment_id: existing.attachment_id,
        });
    }

    create_browser_tab(
        state.inner(),
        metadata.inner(),
        workspace,
        Some(logical_url),
    )
    .await
}

async fn create_browser_tab(
    manager: &BrowserManager,
    metadata: &WorkspaceMetadataManager,
    workspace: String,
    url: Option<String>,
) -> Result<BrowserCreateResult, String> {
    let lookup =
        workspaces::hydrate_workspace_lookup(workspaces::find_workspace(&workspace).await?).await;
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspaces::workspace_not_ready_error(&lookup.workspace));
    }

    let existing_names = lookup
        .workspace
        .sessions()
        .into_iter()
        .map(|session| session.attachment_id)
        .chain(
            manager
                .cache_sessions_for_workspace(&workspace)?
                .into_iter()
                .map(|session| session.attachment_id),
        )
        .collect::<HashSet<_>>();
    let attachment_id = generate_browser_attachment_id(&existing_names);
    let initial_url = manager.resolve_browser_url(&lookup, url.as_deref()).await?;
    let session = browser_session_for_url(
        &attachment_id,
        &initial_url.logical_url,
        Some(BrowserPageMetadata {
            resolved_url: initial_url.resolved_url,
            title: None,
            favicon_url: browser_favicon_for_url(&initial_url.logical_url),
        }),
        None,
        None,
    );
    manager.cache_session(&workspace, session.clone())?;
    metadata.upsert_workspace_session(&workspace, session.clone());
    agent_sessions::upsert_session(&lookup, &session).await?;

    Ok(BrowserCreateResult { attachment_id })
}

#[tauri::command]
pub async fn browser_mount_tab(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
    viewport: BrowserViewport,
    visible: bool,
) -> Result<BrowserMountResult, String> {
    let lookup =
        workspaces::hydrate_workspace_lookup(workspaces::find_workspace(&workspace).await?).await;
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspaces::workspace_not_ready_error(&lookup.workspace));
    }

    let mut session = resolve_browser_session(state.inner(), &lookup, &workspace, &attachment_id)?;
    let resolved_target = state
        .resolve_browser_url(
            &lookup,
            session
                .logical_url
                .as_deref()
                .or(session.url.as_deref())
                .or(session.resolved_url.as_deref()),
        )
        .await?;
    let resolved_url = resolved_target.resolved_url.clone();

    if session.resolved_url.as_deref() != Some(resolved_url.as_str()) {
        session = browser_session_for_url(
            &attachment_id,
            &resolved_target.logical_url,
            Some(BrowserPageMetadata {
                resolved_url: resolved_target.resolved_url.clone(),
                title: session.title.clone(),
                favicon_url: session.favicon_url.clone(),
            }),
            Some(&session),
            session.working,
        );
        state.cache_session(&workspace, session.clone())?;
        metadata
            .inner()
            .upsert_workspace_session(&workspace, session.clone());
        agent_sessions::upsert_session(&lookup, &session).await?;
        emit_browser_state_changed(&app, &workspace, &attachment_id)?;
    }

    app.state::<BrowserLoopbackManager>()
        .cache_workspace_lookup(&lookup);

    state.ensure_webview(
        &app,
        &workspace,
        &attachment_id,
        &resolved_url,
        viewport,
        visible,
    )?;
    state.set_resolved_url(&workspace, &attachment_id, &resolved_url)?;

    Ok(BrowserMountResult {
        attached: true,
        session,
    })
}

#[tauri::command]
pub fn browser_resize_tab(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    workspace: String,
    attachment_id: String,
    viewport: BrowserViewport,
    visible: bool,
) -> Result<BrowserViewportResult, String> {
    let updated =
        state.update_webview_viewport(&app, &workspace, &attachment_id, viewport, visible)?;
    Ok(BrowserViewportResult { updated })
}

#[tauri::command]
pub fn browser_detach_tab(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserDetachResult, String> {
    let detached = state.hide_webview(&app, &workspace, &attachment_id)?;
    Ok(BrowserDetachResult { detached })
}

#[tauri::command]
pub async fn browser_kill_tab(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserKillResult, String> {
    let _ = state.hide_webview(&app, &workspace, &attachment_id)?;
    let _ = state.remove_cached_session(&workspace, &attachment_id)?;
    let cached_sessions = state.cache_sessions_for_workspace(&workspace)?;
    let remaining_sessions = cached_sessions;
    metadata
        .inner()
        .remove_workspace_session(&workspace, BROWSER_KIND, &attachment_id);
    let cleared_active_session = metadata.clear_active_workspace_session_if_matches(
        &workspace,
        BROWSER_KIND,
        &attachment_id,
        None,
    );
    if let Ok(lookup) = workspaces::find_workspace_raw(&workspace).await {
        if let Err(error) =
            agent_sessions::remove_session(&lookup, BROWSER_KIND, &attachment_id).await
        {
            log::warn!(
                "failed to remove browser session from agent workspace={} attachment_id={}: {}",
                workspace,
                attachment_id,
                error
            );
        }
        if cleared_active_session {
            let _ = agent_sessions::set_active_session(&lookup, None).await;
        }
    }
    emit_workspace_state_changed(
        &app,
        &workspace,
        Some((BROWSER_KIND, &attachment_id)),
        cleared_active_session,
        None,
    );

    let manager = state.inner().clone();
    let app_for_cleanup = app.clone();
    let workspace_for_cleanup = workspace.clone();
    let attachment_for_cleanup = attachment_id.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = manager.close_webview(
            &app_for_cleanup,
            &workspace_for_cleanup,
            &attachment_for_cleanup,
        ) {
            log::warn!(
                "failed to close browser webview for workspace {} attachment_id={}: {}",
                workspace_for_cleanup,
                attachment_for_cleanup,
                error
            );
        }
        if let Err(error) = manager
            .loopback_router
            .release_unused_workspace_routes(&workspace_for_cleanup, &remaining_sessions)
        {
            log::warn!(
                "failed to release browser routes for workspace {} after close: {}",
                workspace_for_cleanup,
                error
            );
        }
    });

    Ok(BrowserKillResult { killed: true })
}

#[tauri::command]
pub async fn browser_go_to(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
    url: String,
) -> Result<BrowserCommandResult, String> {
    let existing = find_existing_browser_session(state.inner(), &workspace, &attachment_id).await;
    let lookup = workspaces::find_workspace(&workspace).await?;
    let normalized = state.resolve_browser_url(&lookup, Some(&url)).await?;
    let session = browser_session_for_url(
        &attachment_id,
        &normalized.logical_url,
        Some(BrowserPageMetadata {
            resolved_url: normalized.resolved_url.clone(),
            title: None,
            favicon_url: browser_favicon_for_url(&normalized.logical_url),
        }),
        existing.as_ref(),
        Some(true),
    );
    state.cache_session(&workspace, session.clone())?;
    metadata
        .inner()
        .upsert_workspace_session(&workspace, session.clone());
    agent_sessions::upsert_session(&lookup, &session).await?;

    if let Some(webview) = app.get_webview(&browser_webview_label(&workspace, &attachment_id)) {
        state.set_resolved_url(&workspace, &attachment_id, &normalized.resolved_url)?;
        let destination = Url::parse(&normalized.resolved_url)
            .map_err(|error| format!("invalid browser url: {error}"))?;
        webview
            .navigate(destination)
            .map_err(|error| format!("failed to navigate browser tab: {error}"))?;
    }

    emit_browser_state_changed(&app, &workspace, &attachment_id)?;
    Ok(BrowserCommandResult { updated: true })
}

#[tauri::command]
pub async fn browser_report_page_state(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
    url: String,
    title: Option<String>,
    favicon_url: Option<String>,
) -> Result<BrowserMetadataResult, String> {
    let logical_url = state.logical_url_for_reported_url(&workspace, &url);
    let normalized = normalize_browser_url(Some(&logical_url))?;
    let existing = find_existing_browser_session(state.inner(), &workspace, &attachment_id).await;
    let merged_title = preserve_better_browser_title(
        &normalized.logical_url,
        title,
        existing.as_ref().and_then(|session| session.title.clone()),
    );
    let merged_favicon = preserve_better_browser_favicon(
        &normalized.logical_url,
        favicon_url,
        existing
            .as_ref()
            .and_then(|session| session.favicon_url.clone()),
    );
    state.set_resolved_url(&workspace, &attachment_id, &url)?;
    let session = browser_session_for_url(
        &attachment_id,
        &normalized.logical_url,
        Some(BrowserPageMetadata {
            resolved_url: url,
            title: merged_title,
            favicon_url: merged_favicon,
        }),
        existing.as_ref(),
        None,
    );
    state.cache_session(&workspace, session.clone())?;
    metadata
        .inner()
        .upsert_workspace_session(&workspace, session.clone());
    let lookup = workspaces::find_workspace(&workspace).await?;
    agent_sessions::upsert_session(&lookup, &session).await?;
    emit_browser_state_changed(&app, &workspace, &attachment_id)?;
    Ok(BrowserMetadataResult { updated: true })
}

#[tauri::command]
pub async fn browser_go_back(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserCommandResult, String> {
    run_webview_script(
        &app,
        &workspace,
        &attachment_id,
        "window.history.back();",
        "go back",
    )?;
    set_existing_browser_session_working(
        state.inner(),
        metadata.inner(),
        &app,
        &workspace,
        &attachment_id,
        true,
    )
    .await?;
    Ok(BrowserCommandResult { updated: true })
}

#[tauri::command]
pub async fn browser_go_forward(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserCommandResult, String> {
    run_webview_script(
        &app,
        &workspace,
        &attachment_id,
        "window.history.forward();",
        "go forward",
    )?;
    set_existing_browser_session_working(
        state.inner(),
        metadata.inner(),
        &app,
        &workspace,
        &attachment_id,
        true,
    )
    .await?;
    Ok(BrowserCommandResult { updated: true })
}

#[tauri::command]
pub async fn browser_refresh_page(
    app: AppHandle<AppRuntime>,
    state: State<'_, BrowserManager>,
    metadata: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserCommandResult, String> {
    let webview = resolve_live_webview(&app, &workspace, &attachment_id)?;
    webview
        .reload()
        .map_err(|error| format!("failed to refresh browser tab: {error}"))?;
    set_existing_browser_session_working(
        state.inner(),
        metadata.inner(),
        &app,
        &workspace,
        &attachment_id,
        true,
    )
    .await?;
    Ok(BrowserCommandResult { updated: true })
}

#[tauri::command]
pub async fn browser_open_devtools(
    app: AppHandle<AppRuntime>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserCommandResult, String> {
    let webview = resolve_live_webview(&app, &workspace, &attachment_id)?;
    let _ = webview.set_focus();
    webview.open_devtools();
    Ok(BrowserCommandResult { updated: true })
}

#[tauri::command]
pub async fn browser_toggle_devtools(
    app: AppHandle<AppRuntime>,
    workspace: String,
    attachment_id: String,
) -> Result<BrowserCommandResult, String> {
    let webview = resolve_live_webview(&app, &workspace, &attachment_id)?;
    let devtools_open = webview.is_devtools_open();

    if devtools_open {
        webview.close_devtools();
    } else {
        let _ = webview.set_focus();
        webview.open_devtools();
    }

    Ok(BrowserCommandResult { updated: true })
}

impl BrowserManager {
    pub(crate) fn new(
        loopback_router: RouterManager,
        file_server: BrowserFileServerManager,
    ) -> Self {
        Self {
            webviews: Arc::default(),
            sessions: Arc::default(),
            file_server,
            loopback_router,
        }
    }

    fn cache_session(&self, workspace: &str, session: WorkspaceSession) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "browser session lock poisoned".to_string())?;
        sessions.insert(
            browser_session_cache_key(workspace, &session.attachment_id),
            session,
        );
        Ok(())
    }

    fn remove_cached_session(
        &self,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<Option<WorkspaceSession>, String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "browser session lock poisoned".to_string())?;
        Ok(sessions.remove(&browser_session_cache_key(workspace, attachment_id)))
    }

    fn cached_session(
        &self,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<Option<WorkspaceSession>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "browser session lock poisoned".to_string())?;
        Ok(sessions
            .get(&browser_session_cache_key(workspace, attachment_id))
            .cloned())
    }

    fn cache_sessions_for_workspace(
        &self,
        workspace: &str,
    ) -> Result<Vec<WorkspaceSession>, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| "browser session lock poisoned".to_string())?;
        let mut cached = sessions
            .iter()
            .filter_map(|(key, session)| {
                key.starts_with(&format!("{workspace}:"))
                    .then_some(session.clone())
            })
            .collect::<Vec<_>>();
        terminal::sort_workspace_sessions_oldest_to_newest(&mut cached);
        Ok(cached)
    }

    async fn resolve_browser_url(
        &self,
        lookup: &WorkspaceLookup,
        value: Option<&str>,
    ) -> Result<BrowserUrlTarget, String> {
        let normalized = normalize_browser_url(value)?;
        if normalized.logical_url == BROWSER_DEFAULT_URL
            || normalized.logical_url.starts_with("chrome://")
            || normalized.logical_url.starts_with("about:")
        {
            return Ok(normalized);
        }

        if let Some(resolved_url) = self
            .file_server
            .rewrite_workspace_file_url(&normalized.logical_url)?
        {
            return Ok(BrowserUrlTarget {
                logical_url: normalized.logical_url,
                resolved_url,
            });
        }

        if let Some(resolved_url) = self
            .loopback_router
            .rewrite_loopback_url_async(lookup, &normalized.logical_url)
            .await?
        {
            return Ok(BrowserUrlTarget {
                logical_url: normalized.logical_url,
                resolved_url,
            });
        }

        Ok(normalized)
    }

    fn logical_url_for_reported_url(&self, workspace: &str, resolved_url: &str) -> String {
        self.file_server
            .logical_url_for_resolved_url(resolved_url)
            .or_else(|| {
                self.loopback_router
                    .logical_url_for_reported_url(workspace, resolved_url)
            })
            .unwrap_or_else(|| logical_browser_url(resolved_url))
    }

    fn ensure_webview(
        &self,
        app: &AppHandle<AppRuntime>,
        workspace: &str,
        attachment_id: &str,
        resolved_url: &str,
        viewport: BrowserViewport,
        visible: bool,
    ) -> Result<(), String> {
        let label = browser_webview_label(workspace, attachment_id);
        if let Some(webview) = app.get_webview(&label) {
            set_webview_viewport(&webview, viewport, visible)?;
            if visible {
                let _ = webview.set_focus();
            }
            let current_url = self.current_resolved_url(workspace, attachment_id)?;
            if current_url.as_deref() != Some(resolved_url) {
                let target = Url::parse(resolved_url)
                    .map_err(|error| format!("invalid browser url: {error}"))?;
                webview
                    .navigate(target)
                    .map_err(|error| format!("failed to navigate browser tab: {error}"))?;
                self.set_resolved_url(workspace, attachment_id, resolved_url)?;
            }
            self.upsert_webview_state(
                workspace,
                attachment_id,
                BrowserWebviewState {
                    resolved_url: resolved_url.to_string(),
                    viewport,
                    visible,
                },
            )?;
            return Ok(());
        }

        let window = app
            .get_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "main window not available".to_string())?;
        let destination =
            Url::parse(resolved_url).map_err(|error| format!("invalid browser url: {error}"))?;
        let workspace_name = workspace.to_string();
        let attachment = attachment_id.to_string();
        let app_handle_for_page_load = app.clone();
        let metadata_for_page_load = app.state::<WorkspaceMetadataManager>().inner().clone();
        let app_handle_for_title = app.clone();
        let manager_for_page_load = self.clone();
        let manager_for_title = self.clone();
        let manager_for_popup = self.clone();
        let app_handle_for_popup = app.clone();
        let workspace_for_title = workspace.to_string();
        let attachment_for_title = attachment_id.to_string();
        let workspace_for_popup = workspace.to_string();
        let builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(destination))
            .devtools(true)
            .initialization_script(browser_state_sync_script(workspace, attachment_id))
            .on_page_load(move |webview, payload| {
                handle_page_load(
                    &app_handle_for_page_load,
                    &manager_for_page_load,
                    &metadata_for_page_load,
                    &workspace_name,
                    &attachment,
                    &webview,
                    payload.event(),
                    payload.url().to_string(),
                );
            })
            .on_document_title_changed(move |webview, title| {
                handle_title_changed(
                    &manager_for_title,
                    &app_handle_for_title,
                    &workspace_for_title,
                    &attachment_for_title,
                    &webview,
                    &title,
                );
            })
            .on_new_window(move |url, _features| {
                let popup_url = url.to_string();
                let app_handle = app_handle_for_popup.clone();
                let manager = manager_for_popup.clone();
                let workspace = workspace_for_popup.clone();
                tauri::async_runtime::spawn(async move {
                    let metadata = app_handle.state::<WorkspaceMetadataManager>();
                    match create_browser_tab(
                        &manager,
                        metadata.inner(),
                        workspace.clone(),
                        Some(popup_url),
                    )
                    .await
                    {
                        Ok(result) => {
                            if let Err(error) = emit_browser_popup_created(
                                &app_handle,
                                &workspace,
                                &result.attachment_id,
                            ) {
                                log::warn!(
                                    "failed to emit browser popup created event workspace={} attachment_id={}: {}",
                                    workspace,
                                    result.attachment_id,
                                    error
                                );
                            }
                        }
                        Err(error) => {
                            log::warn!(
                                "failed to create popup browser tab workspace={}: {}",
                                workspace,
                                error
                            );
                        }
                    }
                });
                NewWindowResponse::Deny
            });

        let initial_position = if visible {
            LogicalPosition::new(viewport.x, viewport.y)
        } else {
            LogicalPosition::new(-20_000.0, 0.0)
        };
        let initial_size = if visible {
            LogicalSize::new(viewport.width, viewport.height)
        } else {
            LogicalSize::new(1.0, 1.0)
        };

        window
            .add_child(builder, initial_position, initial_size)
            .map_err(|error| format!("failed to create browser webview: {error}"))?;

        let webview = app
            .get_webview(&label)
            .ok_or_else(|| format!("browser webview missing after creation: {label}"))?;
        set_webview_viewport(&webview, viewport, visible)?;

        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        webviews.insert(
            label.clone(),
            BrowserWebviewState {
                resolved_url: resolved_url.to_string(),
                viewport,
                visible,
            },
        );
        Ok(())
    }

    fn update_webview_viewport(
        &self,
        app: &AppHandle<AppRuntime>,
        workspace: &str,
        attachment_id: &str,
        viewport: BrowserViewport,
        visible: bool,
    ) -> Result<bool, String> {
        if let Some(state) = self.current_webview_state(workspace, attachment_id)? {
            if browser_webview_state_matches_request(&state, viewport, visible) {
                return Ok(true);
            }
        }

        let label = browser_webview_label(workspace, attachment_id);
        let Some(webview) = app.get_webview(&label) else {
            return Ok(false);
        };
        set_webview_viewport(&webview, viewport, visible)?;
        if visible {
            let _ = webview.set_focus();
        }
        self.set_webview_runtime_state(workspace, attachment_id, viewport, visible)?;
        Ok(true)
    }

    fn hide_webview(
        &self,
        app: &AppHandle<AppRuntime>,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<bool, String> {
        if let Some(state) = self.current_webview_state(workspace, attachment_id)? {
            if browser_webview_state_is_hidden(&state) {
                return Ok(true);
            }
        }

        let label = browser_webview_label(workspace, attachment_id);
        let Some(webview) = app.get_webview(&label) else {
            return Ok(false);
        };
        webview
            .hide()
            .map_err(|error| format!("failed to hide browser webview: {error}"))?;
        self.set_webview_visible(workspace, attachment_id, false)?;
        Ok(true)
    }

    fn close_webview(
        &self,
        app: &AppHandle<AppRuntime>,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<bool, String> {
        let label = browser_webview_label(workspace, attachment_id);
        let Some(webview) = app.get_webview(&label) else {
            return Ok(false);
        };
        webview
            .close()
            .map_err(|error| format!("failed to close browser webview: {error}"))?;
        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        webviews.remove(&label);
        Ok(true)
    }

    fn current_resolved_url(
        &self,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<Option<String>, String> {
        let webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        Ok(webviews
            .get(&browser_webview_label(workspace, attachment_id))
            .map(|state| state.resolved_url.clone()))
    }

    fn current_webview_state(
        &self,
        workspace: &str,
        attachment_id: &str,
    ) -> Result<Option<BrowserWebviewState>, String> {
        let webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        Ok(webviews
            .get(&browser_webview_label(workspace, attachment_id))
            .cloned())
    }

    fn upsert_webview_state(
        &self,
        workspace: &str,
        attachment_id: &str,
        state: BrowserWebviewState,
    ) -> Result<(), String> {
        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        webviews.insert(browser_webview_label(workspace, attachment_id), state);
        Ok(())
    }

    fn set_resolved_url(
        &self,
        workspace: &str,
        attachment_id: &str,
        resolved_url: &str,
    ) -> Result<(), String> {
        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        if let Some(state) = webviews.get_mut(&browser_webview_label(workspace, attachment_id)) {
            state.resolved_url = resolved_url.to_string();
        }
        Ok(())
    }

    fn set_webview_runtime_state(
        &self,
        workspace: &str,
        attachment_id: &str,
        viewport: BrowserViewport,
        visible: bool,
    ) -> Result<(), String> {
        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        if let Some(state) = webviews.get_mut(&browser_webview_label(workspace, attachment_id)) {
            state.viewport = viewport;
            state.visible = visible;
        }
        Ok(())
    }

    fn set_webview_visible(
        &self,
        workspace: &str,
        attachment_id: &str,
        visible: bool,
    ) -> Result<(), String> {
        let mut webviews = self
            .webviews
            .lock()
            .map_err(|_| "browser webview lock poisoned".to_string())?;
        if let Some(state) = webviews.get_mut(&browser_webview_label(workspace, attachment_id)) {
            state.visible = visible;
        }
        Ok(())
    }
}

fn browser_session_cache_key(workspace: &str, attachment_id: &str) -> String {
    format!("{workspace}:{attachment_id}")
}

fn browser_webview_state_matches_request(
    state: &BrowserWebviewState,
    viewport: BrowserViewport,
    visible: bool,
) -> bool {
    state.viewport == viewport && state.visible == visible
}

fn browser_webview_state_is_hidden(state: &BrowserWebviewState) -> bool {
    !state.visible
}

fn set_webview_viewport(
    webview: &Webview<AppRuntime>,
    viewport: BrowserViewport,
    visible: bool,
) -> Result<(), String> {
    let bounds = if visible {
        Rect {
            position: Position::Logical(LogicalPosition::new(viewport.x, viewport.y)),
            size: Size::Logical(LogicalSize::new(viewport.width, viewport.height)),
        }
    } else {
        Rect {
            position: Position::Logical(LogicalPosition::new(-20_000.0, 0.0)),
            size: Size::Logical(LogicalSize::new(1.0, 1.0)),
        }
    };
    webview
        .set_bounds(bounds)
        .map_err(|error| format!("failed to update browser viewport: {error}"))?;
    if visible {
        webview
            .show()
            .map_err(|error| format!("failed to show browser webview: {error}"))?;
    } else {
        webview
            .hide()
            .map_err(|error| format!("failed to hide browser webview: {error}"))?;
    }
    Ok(())
}

fn run_webview_script(
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
    script: &str,
    action: &str,
) -> Result<(), String> {
    let webview = resolve_live_webview(app, workspace, attachment_id)?;
    webview
        .eval(script)
        .map_err(|error| format!("failed to {action} in browser tab: {error}"))
}

fn resolve_live_webview(
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
) -> Result<Webview<AppRuntime>, String> {
    let label = browser_webview_label(workspace, attachment_id);
    app.get_webview(&label)
        .ok_or_else(|| format!("browser webview is not mounted: {attachment_id}"))
}

fn handle_page_load(
    app: &AppHandle<AppRuntime>,
    manager: &BrowserManager,
    metadata: &WorkspaceMetadataManager,
    workspace: &str,
    attachment_id: &str,
    webview: &Webview<AppRuntime>,
    event: PageLoadEvent,
    resolved_url: String,
) {
    let _ = manager.set_resolved_url(workspace, attachment_id, &resolved_url);
    if event == PageLoadEvent::Finished {
        reapply_tracked_webview_state(manager, workspace, attachment_id, webview);
        let _ = webview.eval("window.__SILO_BROWSER_SYNC__ && window.__SILO_BROWSER_SYNC__();");
    }

    let logical_url = manager.logical_url_for_reported_url(workspace, &resolved_url);
    let workspace = workspace.to_string();
    let attachment_id = attachment_id.to_string();
    let app_handle = app.clone();
    let metadata_manager = metadata.clone();
    let manager = manager.clone();
    tauri::async_runtime::spawn(async move {
        let existing = find_existing_browser_session(&manager, &workspace, &attachment_id).await;
        let existing_title = existing.as_ref().and_then(|session| session.title.clone());
        let existing_favicon = existing
            .as_ref()
            .and_then(|session| session.favicon_url.clone());
        let session = browser_session_for_url(
            &attachment_id,
            &logical_url,
            Some(BrowserPageMetadata {
                resolved_url,
                title: existing_title,
                favicon_url: existing_favicon.or_else(|| browser_favicon_for_url(&logical_url)),
            }),
            existing.as_ref(),
            Some(event == PageLoadEvent::Started),
        );
        let _ = cache_and_emit_browser_session(
            &manager,
            &metadata_manager,
            &app_handle,
            &workspace,
            &attachment_id,
            session,
        )
        .await;
    });
}

fn handle_title_changed(
    manager: &BrowserManager,
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
    webview: &Webview<AppRuntime>,
    title: &str,
) {
    reapply_tracked_webview_state(manager, workspace, attachment_id, webview);
    let resolved_url = manager
        .current_resolved_url(workspace, attachment_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| BROWSER_DEFAULT_URL.to_string());
    let logical_url = manager.logical_url_for_reported_url(workspace, &resolved_url);
    let workspace = workspace.to_string();
    let attachment_id = attachment_id.to_string();
    let title = title.trim().to_string();
    let app_handle = app.clone();
    let manager = manager.clone();
    tauri::async_runtime::spawn(async move {
        let existing = find_existing_browser_session(&manager, &workspace, &attachment_id).await;
        let merged_title = preserve_better_browser_title(
            &logical_url,
            Some(title),
            existing.as_ref().and_then(|session| session.title.clone()),
        );
        let merged_favicon = preserve_better_browser_favicon(
            &logical_url,
            None,
            existing
                .as_ref()
                .and_then(|session| session.favicon_url.clone()),
        );
        let session = browser_session_for_url(
            &attachment_id,
            &logical_url,
            Some(BrowserPageMetadata {
                resolved_url,
                title: merged_title,
                favicon_url: merged_favicon,
            }),
            existing.as_ref(),
            None,
        );
        let metadata = app_handle.state::<WorkspaceMetadataManager>();
        let _ = cache_and_emit_browser_session(
            &manager,
            metadata.inner(),
            &app_handle,
            &workspace,
            &attachment_id,
            session,
        )
        .await;
    });
}

fn reapply_tracked_webview_state(
    manager: &BrowserManager,
    workspace: &str,
    attachment_id: &str,
    webview: &Webview<AppRuntime>,
) {
    let Ok(Some(state)) = manager.current_webview_state(workspace, attachment_id) else {
        return;
    };
    let _ = set_webview_viewport(webview, state.viewport, state.visible);
}

fn emit_browser_state_changed(
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
) -> Result<(), String> {
    app.emit(
        BROWSER_EVENT_NAME,
        browser_state_event(workspace, attachment_id),
    )
    .map_err(|error| format!("failed to emit browser state event: {error}"))
}

fn emit_browser_popup_created(
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
) -> Result<(), String> {
    app.emit(
        BROWSER_EVENT_NAME,
        browser_popup_created_event(workspace, attachment_id),
    )
    .map_err(|error| format!("failed to emit browser state event: {error}"))
}

fn browser_state_event(workspace: &str, attachment_id: &str) -> BrowserStateEvent {
    BrowserStateEvent {
        workspace: workspace.to_string(),
        attachment_id: attachment_id.to_string(),
        popup_attachment_id: None,
    }
}

fn browser_popup_created_event(workspace: &str, attachment_id: &str) -> BrowserStateEvent {
    BrowserStateEvent {
        workspace: workspace.to_string(),
        attachment_id: attachment_id.to_string(),
        popup_attachment_id: Some(attachment_id.to_string()),
    }
}

fn resolve_browser_session(
    manager: &BrowserManager,
    lookup: &WorkspaceLookup,
    workspace: &str,
    attachment_id: &str,
) -> Result<WorkspaceSession, String> {
    if let Some(session) = manager.cached_session(workspace, attachment_id)? {
        return Ok(session);
    }
    lookup
        .workspace
        .browsers()
        .iter()
        .find(|session| session.attachment_id == attachment_id)
        .cloned()
        .ok_or_else(|| format!("browser session not found: {attachment_id}"))
}

fn browser_webview_label(workspace: &str, attachment_id: &str) -> String {
    format!("browser:{workspace}:{attachment_id}")
}

fn generate_browser_attachment_id(existing_names: &HashSet<String>) -> String {
    let mut timestamp = terminal::current_unix_timestamp_millis();
    loop {
        let candidate = format!("browser-{timestamp}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        timestamp += 1;
    }
}

fn normalize_browser_url(value: Option<&str>) -> Result<BrowserUrlTarget, String> {
    let trimmed = value.unwrap_or_default().trim();
    if trimmed.is_empty() {
        return Ok(BrowserUrlTarget {
            logical_url: BROWSER_DEFAULT_URL.to_string(),
            resolved_url: BROWSER_DEFAULT_URL.to_string(),
        });
    }
    if trimmed == BROWSER_DEFAULT_URL {
        return Ok(BrowserUrlTarget {
            logical_url: BROWSER_DEFAULT_URL.to_string(),
            resolved_url: BROWSER_DEFAULT_URL.to_string(),
        });
    }

    let candidate = if trimmed.contains("://")
        || trimmed.starts_with("about:")
        || trimmed.starts_with("chrome://")
    {
        trimmed.to_string()
    } else if looks_like_browser_address(trimmed) {
        browser_address_with_default_scheme(trimmed)
    } else {
        browser_google_search_url(trimmed)
    };

    if candidate == BROWSER_DEFAULT_URL || candidate.starts_with("chrome://") {
        return Ok(BrowserUrlTarget {
            logical_url: candidate.clone(),
            resolved_url: candidate,
        });
    }

    let parsed = Url::parse(&candidate).map_err(|error| format!("invalid browser url: {error}"))?;
    let logical_url = parsed.to_string();

    Ok(BrowserUrlTarget {
        logical_url,
        resolved_url: parsed.to_string(),
    })
}

fn looks_like_browser_address(value: &str) -> bool {
    if value.chars().any(char::is_whitespace) {
        return false;
    }

    let authority = value.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return false;
    }

    let Some((host, port)) = split_browser_host_and_port(authority) else {
        return false;
    };
    if let Some(port) = port {
        if port.is_empty() || !port.chars().all(|character| character.is_ascii_digit()) {
            return false;
        }
    }

    if host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok() {
        return true;
    }

    if !host
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '.')
    {
        return false;
    }

    let valid_labels = host
        .split('.')
        .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'));
    if !valid_labels {
        return false;
    }

    host.contains('.') || port.is_some()
}

fn split_browser_host_and_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.starts_with('[') {
        let closing = authority.find(']')?;
        let host = &authority[1..closing];
        let remainder = &authority[(closing + 1)..];
        if remainder.is_empty() {
            return Some((host, None));
        }
        return remainder.strip_prefix(':').map(|port| (host, Some(port)));
    }

    if authority.matches(':').count() > 1 {
        return None;
    }

    if let Some((host, port)) = authority.rsplit_once(':') {
        if !host.is_empty() {
            return Some((host, Some(port)));
        }
    }

    Some((authority, None))
}

fn browser_address_with_default_scheme(value: &str) -> String {
    if value.starts_with("localhost")
        || value.starts_with("127.0.0.1")
        || value.starts_with("[::1]")
    {
        return format!("http://{value}");
    }

    format!("https://{value}")
}

fn browser_google_search_url(query: &str) -> String {
    format!(
        "https://www.google.com/search?q={}",
        urlencoding::encode(query)
    )
}

fn logical_browser_url(resolved_url: &str) -> String {
    resolved_url.to_string()
}

fn browser_title_for_url(url: &str) -> String {
    if url == BROWSER_DEFAULT_URL {
        return "browser".to_string();
    }
    if let Some(name) = workspace_file_display_name_from_url(url) {
        return name;
    }
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            return host.to_string();
        }
    }
    url.to_string()
}

fn browser_favicon_for_url(url: &str) -> Option<String> {
    if workspace_file_display_name_from_url(url).is_some() {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let scheme = parsed.scheme();
    let origin = match parsed.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    };
    Some(format!(
        "https://www.google.com/s2/favicons?sz=64&domain_url={}",
        urlencoding::encode(&origin)
    ))
}

fn preserve_better_browser_title(
    logical_url: &str,
    incoming_title: Option<String>,
    existing_title: Option<String>,
) -> Option<String> {
    let fallback_title = browser_title_for_url(logical_url);
    let normalized_incoming = incoming_title.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    });
    let normalized_existing = existing_title.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    });

    match (normalized_incoming, normalized_existing) {
        (Some(incoming), Some(existing))
            if incoming == fallback_title && existing != fallback_title =>
        {
            Some(existing)
        }
        (Some(incoming), _) => Some(incoming),
        (None, existing) => existing,
    }
}

fn preserve_better_browser_favicon(
    logical_url: &str,
    incoming_favicon: Option<String>,
    existing_favicon: Option<String>,
) -> Option<String> {
    let fallback_favicon = browser_favicon_for_url(logical_url);
    let normalized_incoming = incoming_favicon.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    });
    let normalized_existing = existing_favicon.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    });

    match (normalized_incoming, normalized_existing, fallback_favicon) {
        (Some(incoming), Some(existing), Some(fallback))
            if incoming == fallback && existing != fallback =>
        {
            Some(existing)
        }
        (Some(incoming), _, _) => Some(incoming),
        (None, existing, _) => existing,
    }
}

async fn find_existing_browser_session(
    manager: &BrowserManager,
    workspace: &str,
    attachment_id: &str,
) -> Option<WorkspaceSession> {
    if let Ok(Some(session)) = manager.cached_session(workspace, attachment_id) {
        return Some(session);
    }
    let lookup = workspaces::find_workspace(workspace).await.ok()?;
    let lookup = workspaces::hydrate_workspace_lookup(lookup).await;
    lookup
        .workspace
        .browsers()
        .iter()
        .find(|session| session.attachment_id == attachment_id)
        .cloned()
}

fn find_browser_session_by_logical_url(
    manager: &BrowserManager,
    workspace: &str,
    persisted_sessions: &[WorkspaceSession],
    logical_url: &str,
) -> Result<Option<WorkspaceSession>, String> {
    if let Some(session) = manager
        .cache_sessions_for_workspace(workspace)?
        .into_iter()
        .find(|session| {
            session.logical_url.as_deref() == Some(logical_url)
                || session.url.as_deref() == Some(logical_url)
        })
    {
        return Ok(Some(session));
    }

    Ok(persisted_sessions
        .iter()
        .find(|session| {
            session.logical_url.as_deref() == Some(logical_url)
                || session.url.as_deref() == Some(logical_url)
        })
        .cloned())
}

async fn cache_and_emit_browser_session(
    manager: &BrowserManager,
    metadata: &WorkspaceMetadataManager,
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
    session: WorkspaceSession,
) -> Result<(), String> {
    manager.cache_session(workspace, session.clone())?;
    metadata.upsert_workspace_session(workspace, session.clone());
    let lookup = workspaces::find_workspace(workspace).await?;
    if lookup.workspace.is_ready() {
        agent_sessions::upsert_session(&lookup, &session).await?;
    }
    emit_browser_state_changed(app, workspace, attachment_id)?;
    Ok(())
}

async fn set_existing_browser_session_working(
    manager: &BrowserManager,
    metadata: &WorkspaceMetadataManager,
    app: &AppHandle<AppRuntime>,
    workspace: &str,
    attachment_id: &str,
    working: bool,
) -> Result<(), String> {
    let Some(mut session) = find_existing_browser_session(manager, workspace, attachment_id).await
    else {
        return Ok(());
    };

    if session.working == Some(working) {
        return Ok(());
    }

    session.working = Some(working);
    cache_and_emit_browser_session(manager, metadata, app, workspace, attachment_id, session).await
}

struct BrowserPageMetadata {
    resolved_url: String,
    title: Option<String>,
    favicon_url: Option<String>,
}

fn browser_session_for_url(
    attachment_id: &str,
    logical_url: &str,
    metadata: Option<BrowserPageMetadata>,
    existing: Option<&WorkspaceSession>,
    working: Option<bool>,
) -> WorkspaceSession {
    let normalized = normalize_browser_url(Some(logical_url)).unwrap_or(BrowserUrlTarget {
        logical_url: logical_url.to_string(),
        resolved_url: logical_url.to_string(),
    });
    let metadata = metadata.unwrap_or(BrowserPageMetadata {
        resolved_url: normalized.resolved_url.clone(),
        title: None,
        favicon_url: browser_favicon_for_url(&normalized.logical_url),
    });
    let title = metadata
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| browser_title_for_url(&normalized.logical_url));
    WorkspaceSession {
        kind: BROWSER_KIND.to_string(),
        name: title.clone(),
        attachment_id: attachment_id.to_string(),
        path: None,
        url: (normalized.logical_url != BROWSER_DEFAULT_URL)
            .then_some(normalized.logical_url.clone()),
        logical_url: (normalized.logical_url != BROWSER_DEFAULT_URL)
            .then_some(normalized.logical_url),
        resolved_url: (metadata.resolved_url != BROWSER_DEFAULT_URL)
            .then_some(metadata.resolved_url),
        title: Some(title),
        favicon_url: metadata.favicon_url,
        can_go_back: existing.and_then(|session| session.can_go_back),
        can_go_forward: existing.and_then(|session| session.can_go_forward),
        working: working.or(existing.and_then(|session| session.working)),
        unread: existing.and_then(|session| session.unread),
    }
}

fn browser_state_sync_script(workspace: &str, attachment_id: &str) -> String {
    format!(
        r#"
(() => {{
  const workspace = {workspace:?};
  const attachmentId = {attachment_id:?};
  let syncTimer = null;

  const getFavicon = () => {{
    const icon = document.querySelector('link[rel~="icon"][href], link[rel="shortcut icon"][href], link[rel="apple-touch-icon"][href]');
    if (!icon) {{
      return null;
    }}
    try {{
      return new URL(icon.getAttribute('href') || '', window.location.href).toString();
    }} catch (_error) {{
      return null;
    }}
  }};

  const sendState = () => {{
    const invoke =
      window.__TAURI_INTERNALS__?.invoke ||
      window.__TAURI__?.core?.invoke ||
      null;
    if (typeof invoke !== 'function') {{
      return;
    }}
    void invoke('browser_report_page_state', {{
      workspace,
      attachmentId,
      url: window.location.href,
      title: document.title || null,
      faviconUrl: getFavicon(),
    }});
  }};

  const queueSync = () => {{
    if (syncTimer !== null) {{
      window.clearTimeout(syncTimer);
    }}
    syncTimer = window.setTimeout(() => {{
      syncTimer = null;
      sendState();
    }}, 50);
  }};

  window.__SILO_BROWSER_SYNC__ = queueSync;
  document.addEventListener('readystatechange', queueSync);
  window.addEventListener('load', queueSync);
  window.addEventListener('popstate', queueSync);
  window.addEventListener('hashchange', queueSync);

  const titleObserver = new MutationObserver(queueSync);
  const headObserver = new MutationObserver(queueSync);
  const startObservers = () => {{
    if (document.querySelector('title')) {{
      titleObserver.observe(document.querySelector('title'), {{
        subtree: true,
        characterData: true,
        childList: true,
      }});
    }}
    if (document.head) {{
      headObserver.observe(document.head, {{
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ['href', 'rel'],
      }});
    }}
  }};

  const wrapHistory = (method) => {{
    const original = history[method];
    if (typeof original !== 'function') {{
      return;
    }}
    history[method] = function(...args) {{
      const result = original.apply(this, args);
      queueSync();
      return result;
    }};
  }};
  wrapHistory('pushState');
  wrapHistory('replaceState');

  startObservers();
  queueSync();
}})();
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_browser_url_uses_https_for_domains() {
        let normalized = normalize_browser_url(Some("example.com/docs")).expect("should parse");
        assert_eq!(normalized.logical_url, "https://example.com/docs");
        assert_eq!(normalized.resolved_url, "https://example.com/docs");
    }

    #[test]
    fn normalize_browser_url_keeps_localhost_on_http() {
        let normalized = normalize_browser_url(Some("localhost:3000")).expect("should parse");
        assert_eq!(normalized.logical_url, "http://localhost:3000/");
        assert_eq!(normalized.resolved_url, "http://localhost:3000/");
    }

    #[test]
    fn normalize_browser_url_treats_host_and_port_as_address() {
        let normalized = normalize_browser_url(Some("devbox:8080")).expect("should parse");
        assert_eq!(normalized.logical_url, "https://devbox:8080/");
        assert_eq!(normalized.resolved_url, "https://devbox:8080/");
    }

    #[test]
    fn normalize_browser_url_searches_google_for_plain_text() {
        let normalized =
            normalize_browser_url(Some("rust ownership")).expect("search url should parse");
        assert_eq!(
            normalized.logical_url,
            "https://www.google.com/search?q=rust%20ownership"
        );
        assert_eq!(
            normalized.resolved_url,
            "https://www.google.com/search?q=rust%20ownership"
        );
    }

    #[test]
    fn normalize_browser_url_searches_google_for_non_domain_paths() {
        let normalized =
            normalize_browser_url(Some("notes/today")).expect("search url should parse");
        assert_eq!(
            normalized.logical_url,
            "https://www.google.com/search?q=notes%2Ftoday"
        );
        assert_eq!(
            normalized.resolved_url,
            "https://www.google.com/search?q=notes%2Ftoday"
        );
    }

    #[test]
    fn browser_state_event_omits_popup_attachment_id_by_default() {
        let event = serde_json::to_value(browser_state_event("demo", "browser-1"))
            .expect("event should serialize");
        assert_eq!(
            event,
            json!({
                "workspace": "demo",
                "attachmentId": "browser-1",
            })
        );
    }

    #[test]
    fn browser_popup_created_event_includes_popup_attachment_id() {
        let event = serde_json::to_value(browser_popup_created_event("demo", "browser-2"))
            .expect("event should serialize");
        assert_eq!(
            event,
            json!({
                "workspace": "demo",
                "attachmentId": "browser-2",
                "popupAttachmentId": "browser-2",
            })
        );
    }

    #[test]
    fn browser_webview_state_match_requires_same_viewport_and_visibility() {
        let state = BrowserWebviewState {
            resolved_url: "https://example.com".to_string(),
            viewport: BrowserViewport {
                x: 10.0,
                y: 20.0,
                width: 800.0,
                height: 600.0,
            },
            visible: false,
        };

        assert!(browser_webview_state_matches_request(
            &state,
            state.viewport,
            false,
        ));
        assert!(!browser_webview_state_matches_request(
            &state,
            state.viewport,
            true,
        ));
        assert!(!browser_webview_state_matches_request(
            &state,
            BrowserViewport {
                x: 10.0,
                y: 20.0,
                width: 1024.0,
                height: 600.0,
            },
            false,
        ));
        assert!(browser_webview_state_is_hidden(&state));
    }
}
