use crate::agent_sessions;
use crate::bootstrap;
use crate::bootstrap::is_retryable_terminal_transport_error;
use crate::remote::{
    assistant_prompt_command, build_terminal_attach_command, remote_command_error,
    run_remote_command, run_terminal_user_command, shell_quote, terminal_shell_command,
    REMOTE_WORKSPACE_AGENT_BIN,
};
use crate::state::WorkspaceMetadataManager;
use crate::workspaces::{self, WorkspaceActiveSession, WorkspaceLookup, WorkspaceSession};
use crate::{emit_workspace_state_changed, AppRuntime};
use portable_pty::{native_pty_system, Child, ChildKiller, MasterPty, PtySize};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, EventTarget, State, Window};
use uuid::Uuid;

const TERMINAL_EXIT_EVENT: &str = "terminal://exit";
const TERMINAL_ERROR_EVENT: &str = "terminal://error";
const TERMINAL_DISCONNECT_EVENT: &str = "terminal://disconnect";
const MAX_ATTACHMENT_RECENT_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_ATTACHMENT_PENDING_OUTPUT_BYTES: usize = 512 * 1024;
const ATTACH_COMMAND_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_RESERVATION_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_RESERVATION_WAIT_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TerminalAttachMode {
    Fresh,
    Reused,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalAttachResult {
    terminal_id: String,
    session: WorkspaceSession,
    initial_output: Vec<u8>,
    attach_mode: TerminalAttachMode,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalCreateResult {
    pub(crate) attachment_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalRunResult {
    session: WorkspaceSession,
    created: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalDetachResult {
    detached: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalKillResult {
    killed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalReadResult {
    updated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalFinishAttachResult {
    flushed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalProbeResult {
    exists: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TerminalExitPayload {
    terminal_id: String,
    exit_code: u32,
    signal: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TerminalErrorPayload {
    terminal_id: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TerminalDisconnectPayload {
    terminal_id: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AttachmentKey {
    workspace: String,
    name: String,
}

struct Attachment {
    app: Option<AppHandle<AppRuntime>>,
    id: String,
    key: AttachmentKey,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output_state: Mutex<AttachmentOutputState>,
    window_label: Mutex<String>,
    connected: Mutex<bool>,
    connected_cv: Condvar,
    recent_output: Mutex<Vec<u8>>,
}

struct AttachmentOutputState {
    channel: Channel<Vec<u8>>,
    ready: bool,
    pending: Vec<u8>,
}

#[derive(Default)]
struct AttachmentRegistry {
    by_id: HashMap<String, Arc<Attachment>>,
    by_key: HashMap<AttachmentKey, String>,
    reserved_keys: HashSet<AttachmentKey>,
    startup_commands: HashMap<AttachmentKey, String>,
}

#[derive(Clone, Default)]
pub struct TerminalManager {
    inner: Arc<Mutex<AttachmentRegistry>>,
}

struct AttachmentReservation {
    manager: TerminalManager,
    key: AttachmentKey,
}

enum AttachmentSlot {
    Existing(Arc<Attachment>),
    Reserved(AttachmentReservation),
}

impl Drop for AttachmentReservation {
    fn drop(&mut self) {
        self.manager.release_reservation(&self.key);
    }
}

impl TerminalManager {
    fn get_by_key(&self, key: &AttachmentKey) -> Option<Arc<Attachment>> {
        let registry = self.inner.lock().ok()?;
        let id = registry.by_key.get(key)?.clone();
        registry.by_id.get(&id).cloned()
    }

    fn get_by_id(&self, id: &str) -> Option<Arc<Attachment>> {
        self.inner.lock().ok()?.by_id.get(id).cloned()
    }

    fn insert(&self, attachment: Arc<Attachment>) {
        if let Ok(mut registry) = self.inner.lock() {
            registry.reserved_keys.remove(&attachment.key);
            registry
                .by_key
                .insert(attachment.key.clone(), attachment.id.clone());
            registry.by_id.insert(attachment.id.clone(), attachment);
        }
    }

    fn remove_by_id(&self, id: &str) -> Option<Arc<Attachment>> {
        let mut registry = self.inner.lock().ok()?;
        let attachment = registry.by_id.remove(id)?;
        if registry.by_key.get(&attachment.key) == Some(&attachment.id) {
            registry.by_key.remove(&attachment.key);
        }
        Some(attachment)
    }

    fn remove_by_key(&self, key: &AttachmentKey) -> Option<Arc<Attachment>> {
        let id = self.inner.lock().ok()?.by_key.get(key)?.clone();
        self.remove_by_id(&id)
    }

    fn set_startup_command(&self, key: AttachmentKey, command: String) {
        if let Ok(mut registry) = self.inner.lock() {
            registry.startup_commands.insert(key, command);
        }
    }

    fn take_startup_command(&self, key: &AttachmentKey) -> Option<String> {
        self.inner.lock().ok()?.startup_commands.remove(key)
    }

    fn try_reserve(&self, key: &AttachmentKey) -> Option<AttachmentReservation> {
        let mut registry = self.inner.lock().ok()?;
        if registry.by_key.contains_key(key) || registry.reserved_keys.contains(key) {
            return None;
        }

        registry.reserved_keys.insert(key.clone());
        Some(AttachmentReservation {
            manager: self.clone(),
            key: key.clone(),
        })
    }

    fn release_reservation(&self, key: &AttachmentKey) {
        if let Ok(mut registry) = self.inner.lock() {
            registry.reserved_keys.remove(key);
        }
    }
}

#[tauri::command]
pub async fn terminal_create_terminal(
    workspace_state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
) -> Result<TerminalCreateResult, String> {
    let attachment_id =
        create_pending_terminal_session(workspace_state.inner(), &workspace).await?;
    Ok(TerminalCreateResult { attachment_id })
}

pub(crate) async fn ensure_template_startup_terminal_session(
    workspace_state: &WorkspaceMetadataManager,
    workspace: &str,
) -> Result<String, String> {
    let lookup =
        workspaces::hydrate_workspace_lookup(workspaces::find_workspace(workspace).await?).await;
    let workspace_with_state = workspace_state.apply_workspace_state(lookup.workspace);
    let existing_attachment_id = workspace_with_state
        .active_session()
        .filter(|active| {
            active.kind == "terminal"
                && workspace_with_state.has_session("terminal", &active.attachment_id)
        })
        .map(|active| active.attachment_id.clone())
        .or_else(|| {
            workspace_with_state
                .sessions()
                .into_iter()
                .rev()
                .find(|session| session.kind == "terminal")
                .map(|session| session.attachment_id)
        });
    let attachment_id = match existing_attachment_id {
        Some(attachment_id) => attachment_id,
        None => create_pending_terminal_session(workspace_state, workspace).await?,
    };

    let active_session = WorkspaceActiveSession::new("terminal".to_string(), attachment_id.clone());
    workspace_state.set_active_workspace_session(workspace, active_session);
    Ok(attachment_id)
}

#[tauri::command]
pub async fn terminal_create_assistant(
    state: State<'_, TerminalManager>,
    workspace_state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    model: String,
) -> Result<TerminalCreateResult, String> {
    let command = match model.trim() {
        "codex" => "silo codex",
        "claude" => "silo claude",
        other => return Err(format!("unsupported assistant model: {other}")),
    };
    let attachment_id =
        start_terminal_command(state.inner(), workspace_state.inner(), &workspace, command).await?;
    Ok(TerminalCreateResult { attachment_id })
}

#[tauri::command]
pub async fn terminal_list_terminals(workspace: String) -> Result<Vec<WorkspaceSession>, String> {
    log::trace!("listing terminals for workspace {workspace}");
    let lookup = workspaces::find_workspace(&workspace).await?;
    list_terminals_in_workspace(&lookup).await
}

#[tauri::command]
pub async fn terminal_attach_terminal(
    app: AppHandle<AppRuntime>,
    window: Window<AppRuntime>,
    state: State<'_, TerminalManager>,
    workspace: String,
    attachment_id: String,
    cols: u16,
    rows: u16,
    command: Option<String>,
    output: Channel<Vec<u8>>,
) -> Result<TerminalAttachResult, String> {
    let attach_started = Instant::now();
    let lookup = workspaces::find_workspace(&workspace).await?;
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspaces::workspace_not_ready_error(&lookup.workspace));
    }
    log::info!(
        "terminal attach start workspace={} attachment_id={} cols={} rows={}",
        workspace,
        attachment_id,
        cols,
        rows
    );
    let key = AttachmentKey {
        workspace: workspace.clone(),
        name: attachment_id.clone(),
    };
    let startup_command = command.or_else(|| state.take_startup_command(&key));
    let attach_command =
        build_terminal_attach_command(&lookup, &terminal_attach_remote_command(&attachment_id))
            .await?;

    let _reservation = match wait_for_attachment_slot(state.inner(), &key, &attachment_id).await? {
        AttachmentSlot::Existing(existing) => {
            return attach_existing_terminal(
                existing,
                &lookup,
                &attachment_id,
                cols,
                rows,
                &window,
                output,
                startup_command,
                attach_started,
            )
            .await;
        }
        AttachmentSlot::Reserved(reservation) => reservation,
    };

    let spawn_started = Instant::now();
    let attachment = spawn_terminal_attachment(
        app,
        state.inner().clone(),
        key,
        attach_command,
        cols,
        rows,
        output,
        window.label().to_string(),
    )?;
    log::info!(
        "terminal attach spawned pty workspace={} attachment_id={} elapsed_ms={}",
        workspace,
        attachment_id,
        spawn_started.elapsed().as_millis()
    );
    if let Some(command) = startup_command {
        queue_attach_command(attachment.clone(), command);
    }

    log::info!(
        "terminal attach ready workspace={} attachment_id={} elapsed_ms={}",
        workspace,
        attachment_id,
        attach_started.elapsed().as_millis()
    );
    Ok(TerminalAttachResult {
        terminal_id: attachment.id.clone(),
        session: resolve_attached_session(&lookup, &attachment_id).await?,
        initial_output: Vec::new(),
        attach_mode: TerminalAttachMode::Fresh,
    })
}

#[tauri::command]
pub fn terminal_finish_attach(
    state: State<'_, TerminalManager>,
    terminal: String,
) -> Result<TerminalFinishAttachResult, String> {
    let attachment = state
        .get_by_id(&terminal)
        .ok_or_else(|| format!("terminal attachment not found: {terminal}"))?;
    finish_attachment_output(&attachment)?;
    Ok(TerminalFinishAttachResult { flushed: true })
}

#[tauri::command]
pub async fn terminal_run_terminal(
    workspace_state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
    command: String,
) -> Result<TerminalRunResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspaces::workspace_not_ready_error(&lookup.workspace));
    }
    let session = session_for_command(&attachment_id, &command);
    let created = find_terminal_session(&lookup, &attachment_id)
        .await?
        .is_none();
    let result = run_remote_command(
        &lookup,
        &terminal_run_remote_command(&attachment_id, &command),
    )
    .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to run terminal command",
            &result.stderr,
        ));
    }

    workspace_state
        .inner()
        .upsert_workspace_session(&workspace, session.clone());
    Ok(TerminalRunResult { session, created })
}

pub(crate) async fn start_terminal_command(
    manager: &TerminalManager,
    workspace_state: &WorkspaceMetadataManager,
    workspace: &str,
    command: &str,
) -> Result<String, String> {
    let attachment_id = create_terminal_attachment_id(workspace).await?;
    workspace_state.upsert_workspace_session(
        workspace,
        startup_session_for_command(&attachment_id, command),
    );
    manager.set_startup_command(
        AttachmentKey {
            workspace: workspace.to_string(),
            name: attachment_id.clone(),
        },
        command.to_string(),
    );
    Ok(attachment_id)
}

pub(crate) fn codex_prompt_command(prompt: &str) -> String {
    assistant_prompt_command("silo codex", prompt)
}

pub(crate) fn claude_prompt_command(prompt: &str) -> String {
    assistant_prompt_command("silo claude", prompt)
}

#[tauri::command]
pub fn terminal_detach_terminal(
    state: State<'_, TerminalManager>,
    workspace: String,
    attachment_id: String,
) -> Result<TerminalDetachResult, String> {
    let key = AttachmentKey {
        workspace,
        name: attachment_id,
    };
    if let Some(attachment) = state.remove_by_key(&key) {
        kill_local_attachment(&attachment)?;
        return Ok(TerminalDetachResult { detached: true });
    }

    Ok(TerminalDetachResult { detached: false })
}

#[tauri::command]
pub async fn terminal_kill_terminal(
    app: AppHandle<AppRuntime>,
    state: State<'_, TerminalManager>,
    workspace_state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<TerminalKillResult, String> {
    let attachment_id = attachment_id.trim().to_string();
    if attachment_id.is_empty() {
        return Err("terminal attachment_id must not be empty".to_string());
    }

    let key = AttachmentKey {
        workspace: workspace.clone(),
        name: attachment_id.clone(),
    };
    if let Some(attachment) = state.remove_by_key(&key) {
        kill_local_attachment(&attachment)?;
    }

    workspace_state
        .inner()
        .remove_workspace_session(&workspace, "terminal", &attachment_id);
    let cleared_active_session = workspace_state.clear_active_workspace_session_if_matches(
        &workspace,
        "terminal",
        &attachment_id,
        None,
    );
    if cleared_active_session {
        if let Ok(lookup) = workspaces::find_workspace(&workspace).await {
            let _ = agent_sessions::set_active_session(&lookup, None).await;
        }
    }
    emit_workspace_state_changed(
        &app,
        &workspace,
        Some(("terminal", &attachment_id)),
        cleared_active_session,
        None,
    );

    let workspace_for_kill = workspace.clone();
    let attachment_for_kill = attachment_id.clone();
    tauri::async_runtime::spawn(async move {
        let lookup = match workspaces::find_workspace(&workspace_for_kill).await {
            Ok(lookup) => lookup,
            Err(error) => {
                log::warn!(
                    "failed to resolve workspace {} for terminal close {}: {}",
                    workspace_for_kill,
                    attachment_for_kill,
                    error
                );
                return;
            }
        };

        let result = match run_remote_command(
            &lookup,
            &run_terminal_user_command(&format!("zmx kill {}", shell_quote(&attachment_for_kill))),
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                log::warn!(
                    "background terminal kill failed workspace={} attachment_id={}: {}",
                    workspace_for_kill,
                    attachment_for_kill,
                    error
                );
                return;
            }
        };

        if !result.success {
            log::warn!(
                "background terminal kill reported failure workspace={} attachment_id={}: {}",
                workspace_for_kill,
                attachment_for_kill,
                remote_command_error("failed to kill terminal session", &result.stderr)
            );
        }
    });

    Ok(TerminalKillResult { killed: true })
}

#[tauri::command]
pub async fn terminal_read_terminal(
    workspace_state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<TerminalReadResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    let command = run_terminal_user_command(&format!(
        "if [ -x {agent_bin} ]; then {agent_bin} mark-read --session {attachment_id}; fi",
        agent_bin = shell_quote(REMOTE_WORKSPACE_AGENT_BIN),
        attachment_id = shell_quote(&attachment_id),
    ));
    let result = run_remote_command(&lookup, &command).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to mark terminal session as read",
            &result.stderr,
        ));
    }
    let session = find_terminal_session(&lookup, &attachment_id).await?;
    workspace_state.mark_workspace_session_read(&workspace, &attachment_id, session);

    Ok(TerminalReadResult { updated: true })
}

#[tauri::command]
pub fn terminal_write_terminal(
    state: State<'_, TerminalManager>,
    terminal: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let attachment = state
        .get_by_id(&terminal)
        .ok_or_else(|| format!("terminal attachment not found: {terminal}"))?;
    let mut writer = attachment
        .writer
        .lock()
        .map_err(|_| "terminal writer lock poisoned".to_string())?;
    writer
        .write_all(&data)
        .map_err(|error| format!("failed to write terminal input: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush terminal input: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn terminal_resize_terminal(
    state: State<'_, TerminalManager>,
    terminal: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let attachment = state
        .get_by_id(&terminal)
        .ok_or_else(|| format!("terminal attachment not found: {terminal}"))?;
    let master = attachment
        .master
        .lock()
        .map_err(|_| "terminal pty lock poisoned".to_string())?;
    master
        .resize(PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to resize terminal: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn terminal_probe_terminal(
    state: State<'_, TerminalManager>,
    terminal: String,
) -> Result<TerminalProbeResult, String> {
    Ok(probe_terminal_attachment(state.inner(), &terminal))
}

fn resize_attachment(attachment: &Attachment, cols: u16, rows: u16) -> Result<(), String> {
    let master = attachment
        .master
        .lock()
        .map_err(|_| "terminal pty lock poisoned".to_string())?;
    master
        .resize(PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to resize terminal: {error}"))
}

fn spawn_terminal_attachment(
    app: AppHandle<AppRuntime>,
    manager: TerminalManager,
    key: AttachmentKey,
    command: portable_pty::CommandBuilder,
    cols: u16,
    rows: u16,
    output: Channel<Vec<u8>>,
    window_label: String,
) -> Result<Arc<Attachment>, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to create terminal pty: {error}"))?;
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("failed to start terminal attachment: {error}"))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("failed to open terminal reader: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("failed to open terminal writer: {error}"))?;
    let killer = child.clone_killer();
    let attachment = Arc::new(Attachment {
        app: Some(app.clone()),
        id: Uuid::new_v4().to_string(),
        key,
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        output_state: Mutex::new(AttachmentOutputState {
            channel: output,
            ready: false,
            pending: Vec::new(),
        }),
        window_label: Mutex::new(window_label),
        connected: Mutex::new(false),
        connected_cv: Condvar::new(),
        recent_output: Mutex::new(Vec::new()),
    });

    manager.insert(attachment.clone());
    spawn_reader_loop(reader, attachment.clone());
    spawn_waiter_loop(app, manager, attachment.clone(), child);
    Ok(attachment)
}

fn spawn_reader_loop(mut reader: Box<dyn Read + Send>, attachment: Arc<Attachment>) {
    std::thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    mark_attachment_connected(&attachment);
                    let chunk = buffer[..count].to_vec();
                    record_attachment_output(&attachment, &chunk);
                    if let Err(error) = push_attachment_output(&attachment, &chunk) {
                        emit_terminal_error(
                            &attachment,
                            format!("failed to send terminal output: {error}"),
                        );
                        break;
                    }
                }
                Err(error) => {
                    emit_terminal_error(
                        &attachment,
                        format!("failed to read terminal output: {error}"),
                    );
                    break;
                }
            }
        }
    });
}

fn spawn_waiter_loop(
    app: AppHandle<AppRuntime>,
    manager: TerminalManager,
    attachment: Arc<Attachment>,
    mut child: Box<dyn Child + Send + Sync>,
) {
    std::thread::spawn(move || {
        let status = child.wait();
        manager.remove_by_id(&attachment.id);

        match status {
            Ok(status) => {
                let recent_output = recent_attachment_output(&attachment);
                if !status.success() && is_retryable_terminal_transport_error(&recent_output) {
                    emit_terminal_disconnect_with_app(
                        &app,
                        &attachment,
                        remote_command_error("terminal transport disconnected", &recent_output),
                    );
                    return;
                }
                if !status.success() && is_missing_terminal_session_error(&recent_output) {
                    emit_terminal_error_with_app(
                        &app,
                        &attachment,
                        remote_command_error("terminal session unavailable", &recent_output),
                    );
                    return;
                }
                let payload = TerminalExitPayload {
                    terminal_id: attachment.id.clone(),
                    exit_code: status.exit_code(),
                    signal: status.signal().map(ToOwned::to_owned),
                };
                if let Some(window_label) = current_window_label(&attachment) {
                    let _ = app.emit_to(
                        EventTarget::webview_window(window_label),
                        TERMINAL_EXIT_EVENT,
                        payload,
                    );
                }
            }
            Err(error) => emit_terminal_error_with_app(
                &app,
                &attachment,
                format!("terminal attachment wait failed: {error}"),
            ),
        }
    });
}

fn kill_local_attachment(attachment: &Attachment) -> Result<(), String> {
    let mut killer = attachment
        .killer
        .lock()
        .map_err(|_| "terminal killer lock poisoned".to_string())?;
    killer
        .kill()
        .map_err(|error| format!("failed to close terminal attachment: {error}"))
}

fn emit_terminal_error(attachment: &Attachment, message: String) {
    log::warn!("{message}");
    if let Some(window_label) = current_window_label(attachment) {
        if let Some(app) = &attachment.app {
            let _ = app.emit_to(
                EventTarget::webview_window(window_label),
                TERMINAL_ERROR_EVENT,
                TerminalErrorPayload {
                    terminal_id: attachment.id.clone(),
                    message,
                },
            );
        }
    }
}

fn emit_terminal_error_with_app(
    app: &AppHandle<AppRuntime>,
    attachment: &Attachment,
    message: String,
) {
    log::warn!("{message}");
    if let Some(window_label) = current_window_label(attachment) {
        let _ = app.emit_to(
            EventTarget::webview_window(window_label),
            TERMINAL_ERROR_EVENT,
            TerminalErrorPayload {
                terminal_id: attachment.id.clone(),
                message,
            },
        );
    }
}

fn emit_terminal_disconnect_with_app(
    app: &AppHandle<AppRuntime>,
    attachment: &Attachment,
    message: String,
) {
    log::warn!("{message}");
    if let Some(window_label) = current_window_label(attachment) {
        let _ = app.emit_to(
            EventTarget::webview_window(window_label),
            TERMINAL_DISCONNECT_EVENT,
            TerminalDisconnectPayload {
                terminal_id: attachment.id.clone(),
                message,
            },
        );
    }
}

fn current_window_label(attachment: &Attachment) -> Option<String> {
    attachment
        .window_label
        .lock()
        .ok()
        .map(|value| value.clone())
}

fn handoff_attachment_output(
    attachment: &Attachment,
    output: Channel<Vec<u8>>,
    window_label: &str,
) -> Result<Vec<u8>, String> {
    // Lock recent_output before output_state so the replay snapshot and channel
    // handoff happen atomically with respect to the reader loop.
    let recent_output = attachment
        .recent_output
        .lock()
        .map_err(|_| "terminal recent output lock poisoned".to_string())?;
    let initial_output = recent_output.clone();

    let mut output_state = attachment
        .output_state
        .lock()
        .map_err(|_| "terminal output lock poisoned".to_string())?;
    output_state.channel = output;
    output_state.ready = false;
    output_state.pending.clear();
    drop(output_state);
    drop(recent_output);

    let mut current_window = attachment
        .window_label
        .lock()
        .map_err(|_| "terminal window label lock poisoned".to_string())?;
    *current_window = window_label.to_string();
    Ok(initial_output)
}

fn push_attachment_output(attachment: &Attachment, chunk: &[u8]) -> Result<(), String> {
    let mut output_state = attachment
        .output_state
        .lock()
        .map_err(|_| "terminal output lock poisoned".to_string())?;
    if output_state.ready {
        output_state
            .channel
            .send(chunk.to_vec())
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    output_state.pending.extend_from_slice(chunk);
    if output_state.pending.len() > MAX_ATTACHMENT_PENDING_OUTPUT_BYTES {
        let overflow = output_state.pending.len() - MAX_ATTACHMENT_PENDING_OUTPUT_BYTES;
        output_state.pending.drain(..overflow);
    }
    Ok(())
}

fn finish_attachment_output(attachment: &Attachment) -> Result<(), String> {
    let mut output_state = attachment
        .output_state
        .lock()
        .map_err(|_| "terminal output lock poisoned".to_string())?;
    if !output_state.pending.is_empty() {
        let pending = std::mem::take(&mut output_state.pending);
        output_state
            .channel
            .send(pending)
            .map_err(|error| format!("failed to flush terminal output: {error}"))?;
    }
    output_state.ready = true;
    Ok(())
}

fn record_attachment_output(attachment: &Attachment, chunk: &[u8]) {
    if let Ok(mut recent_output) = attachment.recent_output.lock() {
        recent_output.extend_from_slice(chunk);
        if recent_output.len() > MAX_ATTACHMENT_RECENT_OUTPUT_BYTES {
            let overflow = recent_output.len() - MAX_ATTACHMENT_RECENT_OUTPUT_BYTES;
            recent_output.drain(..overflow);
        }
    }
}

fn probe_terminal_attachment(manager: &TerminalManager, terminal: &str) -> TerminalProbeResult {
    TerminalProbeResult {
        exists: manager.get_by_id(terminal).is_some(),
    }
}

fn recent_attachment_output(attachment: &Attachment) -> String {
    attachment
        .recent_output
        .lock()
        .map(|buffer| String::from_utf8_lossy(&buffer).into_owned())
        .unwrap_or_default()
}

fn mark_attachment_connected(attachment: &Attachment) {
    if let Ok(mut connected) = attachment.connected.lock() {
        if !*connected {
            *connected = true;
            attachment.connected_cv.notify_all();
        }
    }
}

fn queue_attach_command(attachment: Arc<Attachment>, command: String) {
    let data = terminal_command_bytes(&command);
    if data.is_empty() {
        return;
    }

    std::thread::spawn(move || {
        if let Ok(connected) = attachment.connected.lock() {
            let _ = attachment.connected_cv.wait_timeout_while(
                connected,
                ATTACH_COMMAND_WAIT_TIMEOUT,
                |is_connected| !*is_connected,
            );
        }

        if let Err(error) = write_attachment_input(&attachment, &data) {
            emit_terminal_error(
                &attachment,
                format!("failed to send attach command to terminal: {error}"),
            );
        }
    });
}

async fn attach_existing_terminal(
    existing: Arc<Attachment>,
    lookup: &WorkspaceLookup,
    name: &str,
    cols: u16,
    rows: u16,
    window: &Window<AppRuntime>,
    output: Channel<Vec<u8>>,
    command: Option<String>,
    attach_started: Instant,
) -> Result<TerminalAttachResult, String> {
    resize_attachment(&existing, cols, rows)?;
    let initial_output = handoff_attachment_output(&existing, output, window.label().as_ref())?;
    if let Some(command) = command {
        queue_attach_command(existing.clone(), command);
    }

    log::info!(
        "terminal attach reused existing pty workspace={} attachment_id={} elapsed_ms={}",
        lookup.workspace.name(),
        name,
        attach_started.elapsed().as_millis()
    );
    Ok(TerminalAttachResult {
        terminal_id: existing.id.clone(),
        session: resolve_attached_session(lookup, name).await?,
        initial_output,
        attach_mode: TerminalAttachMode::Reused,
    })
}

async fn wait_for_attachment_slot(
    manager: &TerminalManager,
    key: &AttachmentKey,
    name: &str,
) -> Result<AttachmentSlot, String> {
    let started = std::time::Instant::now();
    loop {
        if let Some(existing) = manager.get_by_key(key) {
            return Ok(AttachmentSlot::Existing(existing));
        }
        if let Some(reservation) = manager.try_reserve(key) {
            return Ok(AttachmentSlot::Reserved(reservation));
        }
        if started.elapsed() >= ATTACH_RESERVATION_WAIT_TIMEOUT {
            return Err(format!("terminal attachment already in progress: {name}"));
        }
        std::thread::sleep(ATTACH_RESERVATION_WAIT_INTERVAL);
    }
}

fn write_attachment_input(attachment: &Attachment, data: &[u8]) -> Result<(), String> {
    let mut writer = attachment
        .writer
        .lock()
        .map_err(|_| "terminal writer lock poisoned".to_string())?;
    writer
        .write_all(data)
        .map_err(|error| format!("failed to write terminal input: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("failed to flush terminal input: {error}"))
}

pub(crate) async fn list_terminals_in_workspace(
    lookup: &WorkspaceLookup,
) -> Result<Vec<WorkspaceSession>, String> {
    let mut sessions = list_live_terminals_in_workspace(lookup).await?;
    sort_workspace_sessions_oldest_to_newest(&mut sessions);
    Ok(sessions)
}

async fn list_live_terminals_in_workspace(
    lookup: &WorkspaceLookup,
) -> Result<Vec<WorkspaceSession>, String> {
    if !lookup.workspace.is_ready() {
        return Ok(lookup.workspace.terminals().to_vec());
    }

    let command = run_terminal_user_command(&format!(
        "if [ -x {agent_bin} ]; then {agent_bin} terminals; else printf '[]'; fi",
        agent_bin = shell_quote(REMOTE_WORKSPACE_AGENT_BIN),
    ));
    let result = run_remote_command(lookup, &command).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to list live terminal sessions",
            &result.stderr,
        ));
    }

    parse_live_terminal_sessions(&result.stdout)
}

fn parse_live_terminal_sessions(stdout: &str) -> Result<Vec<WorkspaceSession>, String> {
    let stdout = stdout.trim();
    if stdout.is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse live terminal sessions: {error}"))
}

async fn find_terminal_session(
    lookup: &WorkspaceLookup,
    attachment_id: &str,
) -> Result<Option<WorkspaceSession>, String> {
    Ok(list_terminals_in_workspace(lookup)
        .await?
        .into_iter()
        .find(|session| session.attachment_id == attachment_id))
}

async fn resolve_attached_session(
    lookup: &WorkspaceLookup,
    attachment_id: &str,
) -> Result<WorkspaceSession, String> {
    for _ in 0..5 {
        if let Some(session) = find_terminal_session(lookup, attachment_id).await? {
            return Ok(session);
        }

        std::thread::sleep(Duration::from_millis(150));
    }

    Ok(pending_terminal_session(attachment_id))
}

async fn create_pending_terminal_session(
    workspace_state: &WorkspaceMetadataManager,
    workspace: &str,
) -> Result<String, String> {
    log::trace!("creating terminal attachment id for workspace {workspace}");
    let attachment_id = create_terminal_attachment_id(workspace).await?;
    workspace_state.upsert_workspace_session(workspace, pending_terminal_session(&attachment_id));
    Ok(attachment_id)
}

async fn create_terminal_attachment_id(workspace: &str) -> Result<String, String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    let existing_names = list_terminals_in_workspace(&lookup)
        .await?
        .into_iter()
        .map(|session| session.attachment_id)
        .collect::<HashSet<_>>();
    Ok(generate_terminal_attachment_id(&existing_names))
}

fn pending_terminal_session(attachment_id: &str) -> WorkspaceSession {
    WorkspaceSession {
        kind: "terminal".to_string(),
        name: "shell".to_string(),
        attachment_id: attachment_id.to_string(),
        path: None,
        url: None,
        logical_url: None,
        resolved_url: None,
        title: None,
        favicon_url: None,
        can_go_back: None,
        can_go_forward: None,
        working: None,
        unread: None,
    }
}

fn generate_terminal_attachment_id(existing_names: &HashSet<String>) -> String {
    let mut timestamp = current_unix_timestamp_millis();
    loop {
        let candidate = format!("terminal-{timestamp}");
        if !existing_names.contains(&candidate) {
            return candidate;
        }
        timestamp += 1;
    }
}

pub(crate) fn current_unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub(crate) fn sort_workspace_sessions_oldest_to_newest(sessions: &mut [WorkspaceSession]) {
    sessions.sort_by(|left, right| {
        session_name_timestamp(&left.attachment_id)
            .cmp(&session_name_timestamp(&right.attachment_id))
            .then_with(|| left.attachment_id.cmp(&right.attachment_id))
    });
}

fn session_name_timestamp(name: &str) -> Option<u128> {
    if let Some(timestamp) = name.strip_prefix("terminal-") {
        return timestamp.parse::<u128>().ok();
    }
    if let Some(timestamp) = name.strip_prefix("browser-") {
        return timestamp.parse::<u128>().ok();
    }
    if let Some(timestamp) = name.strip_prefix("file-") {
        return timestamp.parse::<u128>().ok();
    }
    None
}

fn session_for_command(attachment_id: &str, command: &str) -> WorkspaceSession {
    let name = sanitize_session_display_name(command);
    let assistant_capable = assistant_capable_command(&name);
    WorkspaceSession {
        kind: "terminal".to_string(),
        name,
        attachment_id: attachment_id.to_string(),
        path: None,
        url: None,
        logical_url: None,
        resolved_url: None,
        title: None,
        favicon_url: None,
        can_go_back: None,
        can_go_forward: None,
        working: assistant_capable.then_some(false),
        unread: assistant_capable.then_some(false),
    }
}

fn startup_session_for_command(attachment_id: &str, command: &str) -> WorkspaceSession {
    let mut session = session_for_command(attachment_id, command);
    session.working = None;
    session.unread = None;
    session
}

fn sanitize_session_display_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "shell".to_string();
    }
    if trimmed.starts_with("silo codex \"$(printf %s ") && trimmed.ends_with("| base64 --decode)\"")
    {
        return "codex".to_string();
    }
    if trimmed.starts_with("silo claude \"$(printf %s ")
        && trimmed.ends_with("| base64 --decode)\"")
    {
        return "claude".to_string();
    }
    trimmed.chars().take(200).collect()
}

fn assistant_capable_command(command: &str) -> bool {
    let token = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(token.as_str(), "codex" | "claude" | "cc")
}

fn terminal_attach_remote_command(name: &str) -> String {
    run_terminal_user_command(&terminal_shell_command(&format!(
        "exec zmx attach {}",
        shell_quote(name)
    )))
}

fn terminal_run_remote_command(name: &str, command: &str) -> String {
    let command = format!("zmx run {} {}", shell_quote(name), shell_quote(command));
    run_terminal_user_command(&terminal_shell_command(&command))
}

fn is_missing_terminal_session_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("session not found")
        || lower.contains("no such session")
        || lower.contains("unknown session")
}

fn terminal_command_bytes(command: &str) -> Vec<u8> {
    if command.is_empty() {
        return Vec::new();
    }

    let mut data = command.as_bytes().to_vec();
    if !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use portable_pty::{native_pty_system, ChildKiller};
    use serde_json::json;
    use std::collections::HashSet;
    use std::sync::{Arc, Condvar, Mutex};
    use tauri::ipc::{Channel, InvokeResponseBody};

    #[test]
    fn terminal_attach_mode_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_value(TerminalAttachMode::Fresh).expect("serialize attach mode"),
            json!("fresh")
        );
        assert_eq!(
            serde_json::to_value(TerminalAttachMode::Reused).expect("serialize attach mode"),
            json!("reused")
        );
    }

    #[test]
    fn terminal_run_remote_command_passes_command_as_argument() {
        assert_eq!(
            terminal_run_remote_command("terminal-1", "codex -- \"hello\""),
            "sudo -iu silo bash -lc 'if [ -f '\"'\"'/home/silo/.silo/credentials.sh'\"'\"' ]; then source '\"'\"'/home/silo/.silo/credentials.sh'\"'\"'; fi; cd '\"'\"'/home/silo/workspace'\"'\"'; zmx run '\"'\"'terminal-1'\"'\"' '\"'\"'codex -- \"hello\"'\"'\"''"
        );
    }

    #[test]
    fn codex_prompt_command_encodes_multiline_prompt_on_one_line() {
        let prompt = "what is this project?\ninclude 'quotes' too";
        let command = codex_prompt_command(prompt);

        assert!(command.starts_with("silo codex \"$(printf %s '"));
        assert!(command.ends_with("| base64 --decode)\""));

        let encoded = command
            .strip_prefix("silo codex \"$(printf %s '")
            .and_then(|value| value.strip_suffix("' | base64 --decode)\""))
            .expect("command should embed a base64 prompt");
        let decoded = String::from_utf8(
            BASE64_STANDARD
                .decode(encoded)
                .expect("embedded prompt should decode"),
        )
        .expect("embedded prompt should be utf8");
        assert_eq!(decoded, prompt);
    }

    #[test]
    fn claude_prompt_command_encodes_multiline_prompt_on_one_line() {
        let prompt = "ship the change\ninclude 'quotes' too";
        let command = claude_prompt_command(prompt);

        assert!(command.starts_with("silo claude \"$(printf %s '"));
        assert!(command.ends_with("| base64 --decode)\""));

        let encoded = command
            .strip_prefix("silo claude \"$(printf %s '")
            .and_then(|value| value.strip_suffix("' | base64 --decode)\""))
            .expect("command should embed a base64 prompt");
        let decoded = String::from_utf8(
            BASE64_STANDARD
                .decode(encoded)
                .expect("embedded prompt should decode"),
        )
        .expect("embedded prompt should be utf8");
        assert_eq!(decoded, prompt);
    }

    #[test]
    fn terminal_command_bytes_appends_newline_once() {
        assert_eq!(terminal_command_bytes("pwd"), b"pwd\n");
        assert_eq!(terminal_command_bytes("pwd\n"), b"pwd\n");
        assert!(terminal_command_bytes("").is_empty());
    }

    #[test]
    fn generate_terminal_attachment_id_avoids_existing_names() {
        let existing = HashSet::from([
            "terminal-1741812345678".to_string(),
            "terminal-1741812345679".to_string(),
        ]);
        let generated = generate_terminal_attachment_id(&existing);
        assert!(generated.starts_with("terminal-"));
        assert!(!existing.contains(&generated));
        assert!(generated["terminal-".len()..].parse::<u128>().is_ok());
    }

    #[test]
    fn sort_workspace_sessions_orders_oldest_to_newest_by_attachment_id() {
        let mut sessions = vec![
            WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
                attachment_id: "terminal-1741812345680".to_string(),
                path: None,
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            },
            WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
                attachment_id: "terminal-1741812345678".to_string(),
                path: None,
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            },
            WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
                attachment_id: "terminal-1741812345679".to_string(),
                path: None,
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            },
        ];

        sort_workspace_sessions_oldest_to_newest(&mut sessions);

        assert_eq!(sessions[0].attachment_id, "terminal-1741812345678");
        assert_eq!(sessions[1].attachment_id, "terminal-1741812345679");
        assert_eq!(sessions[2].attachment_id, "terminal-1741812345680");
    }

    #[test]
    fn session_for_command_maps_assistant_state() {
        let session = session_for_command("terminal-1", "codex");
        assert_eq!(session.kind, "terminal");
        assert_eq!(session.name, "codex");
        assert_eq!(session.attachment_id, "terminal-1");
        assert_eq!(session.working, Some(false));
        assert_eq!(session.unread, Some(false));
    }

    #[test]
    fn session_for_command_defaults_shell_for_empty_input() {
        let session = session_for_command("terminal-1", "   ");
        assert_eq!(session.name, "shell");
        assert_eq!(session.working, None);
        assert_eq!(session.unread, None);
    }

    #[test]
    fn session_for_command_normalizes_generated_claude_prompt_name() {
        let session = session_for_command(
            "terminal-1",
            "silo claude \"$(printf %s 'c2hpcCBpdA==' | base64 --decode)\"",
        );
        assert_eq!(session.name, "claude");
        assert_eq!(session.working, Some(false));
        assert_eq!(session.unread, Some(false));
    }

    #[test]
    fn startup_session_for_command_defers_assistant_state_to_agent() {
        let session = startup_session_for_command("terminal-1", "codex");
        assert_eq!(session.name, "codex");
        assert_eq!(session.working, None);
        assert_eq!(session.unread, None);
    }

    #[test]
    fn parse_live_terminal_sessions_accepts_agent_published_shape() {
        let sessions = parse_live_terminal_sessions(
            r#"
            [
              {
                "type": "terminal",
                "name": "codex",
                "attachment_id": "terminal-1",
                "working": true,
                "unread": false
              }
            ]
            "#,
        )
        .expect("agent terminal payload should parse");

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].kind, "terminal");
        assert_eq!(sessions[0].name, "codex");
        assert_eq!(sessions[0].attachment_id, "terminal-1");
        assert_eq!(sessions[0].working, Some(true));
        assert_eq!(sessions[0].unread, Some(false));
    }

    #[test]
    fn terminal_manager_round_trip_by_key_and_id() {
        let manager = TerminalManager::default();
        let key = AttachmentKey {
            workspace: "ws".to_string(),
            name: "dev".to_string(),
        };
        let attachment = Arc::new(Attachment {
            app: None,
            id: "att-1".to_string(),
            key: key.clone(),
            master: Mutex::new(
                native_pty_system()
                    .openpty(PtySize::default())
                    .expect("pty")
                    .master,
            ),
            writer: Mutex::new(Box::new(Vec::<u8>::new())),
            killer: Mutex::new(Box::new(NoopKiller)),
            output_state: Mutex::new(AttachmentOutputState {
                channel: Channel::new(|_| Ok(())),
                ready: false,
                pending: Vec::new(),
            }),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
            recent_output: Mutex::new(Vec::new()),
        });
        manager.insert(attachment.clone());

        assert!(manager.get_by_key(&key).is_some());
        assert!(manager.get_by_id("att-1").is_some());
        assert!(manager.remove_by_id("att-1").is_some());
        assert!(manager.get_by_key(&key).is_none());
    }

    #[test]
    fn terminal_manager_reservation_blocks_duplicate_claims() {
        let manager = TerminalManager::default();
        let key = AttachmentKey {
            workspace: "ws".to_string(),
            name: "dev".to_string(),
        };

        let reservation = manager
            .try_reserve(&key)
            .expect("first reservation should succeed");
        assert!(manager.try_reserve(&key).is_none());

        drop(reservation);

        assert!(manager.try_reserve(&key).is_some());
    }

    #[test]
    fn terminal_manager_insert_clears_matching_reservation() {
        let manager = TerminalManager::default();
        let key = AttachmentKey {
            workspace: "ws".to_string(),
            name: "dev".to_string(),
        };

        let reservation = manager
            .try_reserve(&key)
            .expect("reservation should succeed");
        let attachment = Arc::new(Attachment {
            app: None,
            id: "att-1".to_string(),
            key: key.clone(),
            master: Mutex::new(
                native_pty_system()
                    .openpty(PtySize::default())
                    .expect("pty")
                    .master,
            ),
            writer: Mutex::new(Box::new(Vec::<u8>::new())),
            killer: Mutex::new(Box::new(NoopKiller)),
            output_state: Mutex::new(AttachmentOutputState {
                channel: Channel::new(|_| Ok(())),
                ready: false,
                pending: Vec::new(),
            }),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
            recent_output: Mutex::new(Vec::new()),
        });

        manager.insert(attachment.clone());
        drop(reservation);

        assert_eq!(
            manager
                .get_by_key(&key)
                .expect("attachment should be present")
                .id,
            "att-1"
        );
        assert!(manager.try_reserve(&key).is_none());
    }

    #[test]
    fn handoff_attachment_output_replays_recent_output_and_resets_pending_gap() {
        let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let sent_clone = Arc::clone(&sent);
        let attachment = Arc::new(Attachment {
            app: None,
            id: "att-1".to_string(),
            key: AttachmentKey {
                workspace: "ws".to_string(),
                name: "dev".to_string(),
            },
            master: Mutex::new(
                native_pty_system()
                    .openpty(PtySize::default())
                    .expect("pty")
                    .master,
            ),
            writer: Mutex::new(Box::new(Vec::<u8>::new())),
            killer: Mutex::new(Box::new(NoopKiller)),
            output_state: Mutex::new(AttachmentOutputState {
                channel: Channel::new(|_| Ok(())),
                ready: true,
                pending: b"stale-pending".to_vec(),
            }),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
            recent_output: Mutex::new(b"prompt\r\nstale-pending".to_vec()),
        });
        let replay_channel = Channel::new(move |payload| {
            let bytes = match payload {
                InvokeResponseBody::Raw(bytes) => bytes,
                InvokeResponseBody::Json(value) => {
                    serde_json::from_str(&value).expect("json channel payload should decode")
                }
            };
            sent_clone.lock().expect("sent output lock").push(bytes);
            Ok(())
        });

        let initial_output =
            handoff_attachment_output(&attachment, replay_channel, "secondary").expect("handoff");
        assert_eq!(initial_output, b"prompt\r\nstale-pending".to_vec());
        assert_eq!(
            attachment
                .window_label
                .lock()
                .expect("window label lock")
                .as_str(),
            "secondary"
        );
        {
            let output_state = attachment.output_state.lock().expect("output state lock");
            assert!(!output_state.ready);
            assert!(output_state.pending.is_empty());
        }

        push_attachment_output(&attachment, b"attach-gap").expect("buffer gap output");
        finish_attachment_output(&attachment).expect("flush attach gap");
        assert_eq!(
            sent.lock().expect("sent output lock").as_slice(),
            &[b"attach-gap".to_vec()]
        );
    }

    #[test]
    fn terminal_manager_startup_command_is_consumed_once() {
        let manager = TerminalManager::default();
        let key = AttachmentKey {
            workspace: "ws".to_string(),
            name: "dev".to_string(),
        };

        manager.set_startup_command(key.clone(), "codex -- \"hello\"".to_string());

        assert_eq!(
            manager.take_startup_command(&key),
            Some("codex -- \"hello\"".to_string())
        );
        assert_eq!(manager.take_startup_command(&key), None);
    }

    #[test]
    fn terminal_probe_attachment_reports_presence() {
        let manager = TerminalManager::default();
        let key = AttachmentKey {
            workspace: "ws".to_string(),
            name: "dev".to_string(),
        };

        assert_eq!(
            probe_terminal_attachment(&manager, "att-1"),
            TerminalProbeResult { exists: false }
        );

        let attachment = Arc::new(Attachment {
            app: None,
            id: "att-1".to_string(),
            key: key.clone(),
            master: Mutex::new(
                native_pty_system()
                    .openpty(PtySize::default())
                    .expect("pty")
                    .master,
            ),
            writer: Mutex::new(Box::new(Vec::<u8>::new())),
            killer: Mutex::new(Box::new(NoopKiller)),
            output_state: Mutex::new(AttachmentOutputState {
                channel: Channel::new(|_| Ok(())),
                ready: false,
                pending: Vec::new(),
            }),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
            recent_output: Mutex::new(Vec::new()),
        });

        manager.insert(attachment);

        assert_eq!(
            probe_terminal_attachment(&manager, "att-1"),
            TerminalProbeResult { exists: true }
        );

        manager.remove_by_key(&key);

        assert_eq!(
            probe_terminal_attachment(&manager, "att-1"),
            TerminalProbeResult { exists: false }
        );
    }

    #[derive(Debug)]
    struct NoopKiller;

    impl ChildKiller for NoopKiller {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
            Box::new(NoopKiller)
        }
    }
}
