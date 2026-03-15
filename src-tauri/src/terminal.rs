use crate::config::ConfigStore;
use crate::workspaces::{self, WorkspaceLookup, WorkspaceSession};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, EventTarget, State, WebviewWindow};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

const TERMINAL_EXIT_EVENT: &str = "terminal://exit";
const TERMINAL_ERROR_EVENT: &str = "terminal://error";
const DEFAULT_TERMINAL_COLS: u16 = 80;
const DEFAULT_TERMINAL_ROWS: u16 = 24;
const MAX_SCROLLBACK_BYTES: usize = 512 * 1024;
const ATTACH_COMMAND_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_RESERVATION_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_RESERVATION_WAIT_INTERVAL: Duration = Duration::from_millis(50);
const TERMINAL_USER: &str = "silo";
pub(crate) const TERMINAL_WORKSPACE_DIR: &str = "/home/silo/workspace";
const REMOTE_CREDENTIALS_FILE: &str = "/home/silo/.silo/credentials.sh";
const REMOTE_BOOTSTRAP_STATE_FILE: &str = "/home/silo/.silo/workspace-bootstrap-state";
const REMOTE_BOOTSTRAP_LOCK_DIR: &str = "/home/silo/.silo/workspace-bootstrap.lock";
const REMOTE_CHROME_USER_DATA_DIR: &str = "/home/silo/.config/google-chrome";
const REMOTE_WORKSPACE_OBSERVER_BIN: &str = "/home/silo/.silo/bin/workspace-observer";
const REMOTE_WORKSPACE_OBSERVER_PIDFILE: &str = "/home/silo/.silo/workspace-observer/daemon.pid";
const REMOTE_WORKSPACE_OBSERVER_SHELL_FILE: &str = "/home/silo/.silo/workspace-observer-shell.sh";
const WORKSPACE_BOOTSTRAP_VERSION: &str = "10";
const TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS: usize = 60;
const TEMPLATE_BOOTSTRAP_POLL_INTERVAL: Duration = Duration::from_secs(2);
const WORKSPACE_OBSERVER_BIN_BYTES: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/workspace-observer-x86_64-unknown-linux-musl"
));
const CHROME_PROFILE_SYNC_PATHS: &[&str] = &[
    "Local State",
    "First Run",
    "Last Version",
    "Default/Preferences",
    "Default/Secure Preferences",
    "Default/Bookmarks",
    "Default/Bookmarks.bak",
    "Default/History",
    "Default/History-journal",
    "Default/Favicons",
    "Default/Favicons-journal",
    "Default/Web Data",
    "Default/Web Data-journal",
    "Default/Login Data",
    "Default/Login Data-journal",
    "Default/Login Data For Account",
    "Default/Cookies",
    "Default/Cookies-journal",
    "Default/Network Persistent State",
    "Default/Account Web Data",
    "Default/Affiliation Database",
    "Default/Session Storage",
    "Default/Local Storage",
    "Default/IndexedDB",
    "Default/Service Worker",
    "Default/Accounts",
    "Default/trusted_vault.pb",
];

static TEMPLATE_CHROME_SYNC_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static TEMPLATE_BOOTSTRAP_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static WORKSPACE_SSH_READY_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalAttachResult {
    terminal_id: String,
    session: WorkspaceSession,
    scrollback_vt: String,
    scrollback_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalCreateResult {
    attachment_id: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AttachmentKey {
    workspace: String,
    name: String,
}

struct Attachment {
    app: Option<AppHandle>,
    id: String,
    key: AttachmentKey,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<Channel<Vec<u8>>>,
    window_label: Mutex<String>,
    connected: Mutex<bool>,
    connected_cv: Condvar,
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

#[derive(Debug)]
pub(crate) struct CommandResult {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug, Clone)]
struct WorkspaceBootstrap {
    remote_url: String,
    target_branch: String,
    workspace_branch: Option<String>,
    gh_username: String,
    gh_token: String,
    chrome_user_data_dir: String,
    chrome_user_data_hash: String,
    codex_token: String,
    claude_token: String,
    git_user_name: String,
    git_user_email: String,
    env_files: Vec<BootstrapEnvFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BootstrapEnvFile {
    relative_path: String,
    contents_base64: String,
    contents_sha256: String,
}

#[tauri::command]
pub async fn terminal_create_terminal(workspace: String) -> Result<TerminalCreateResult, String> {
    log::trace!("creating terminal attachment id for workspace {workspace}");
    let lookup = workspaces::find_workspace(&workspace).await?;
    let existing_names = list_terminals_in_workspace(&lookup)
        .await?
        .into_iter()
        .map(|session| session.attachment_id)
        .collect::<HashSet<_>>();
    let attachment_id = generate_terminal_attachment_id(&existing_names);
    let session = WorkspaceSession {
        kind: "terminal".to_string(),
        name: "shell".to_string(),
        attachment_id: attachment_id.clone(),
        working: None,
        unread: None,
    };
    write_workspace_terminal_metadata(
        &workspace,
        &lookup,
        mutate_sessions(
            lookup.workspace.sessions(),
            lookup.workspace.last_working(),
            SessionMutation::Upsert(session.clone()),
        ),
    )
    .await?;

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
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, TerminalManager>,
    workspace: String,
    attachment_id: String,
    skip_scrollback: Option<bool>,
    command: Option<String>,
    output: Channel<Vec<u8>>,
) -> Result<TerminalAttachResult, String> {
    let attach_started = Instant::now();
    let lookup = workspaces::find_workspace(&workspace).await?;
    if !lookup.workspace.ready() {
        return Err(format!("workspace {workspace} is not ready"));
    }
    let scrollback_mode = attach_scrollback_mode(skip_scrollback);
    log::info!(
        "terminal attach start workspace={} attachment_id={} skip_scrollback={}",
        workspace,
        attachment_id,
        matches!(scrollback_mode, AttachScrollbackMode::Skip)
    );
    let key = AttachmentKey {
        workspace: workspace.clone(),
        name: attachment_id.clone(),
    };
    let startup_command = command.or_else(|| state.take_startup_command(&key));
    if let Some(command) = startup_command.as_deref() {
        let session = session_for_command(&attachment_id, command);
        write_workspace_terminal_metadata(
            &workspace,
            &lookup,
            mutate_sessions(
                lookup.workspace.sessions(),
                lookup.workspace.last_working(),
                SessionMutation::Upsert(session),
            ),
        )
        .await?;
    }

    let _reservation = match wait_for_attachment_slot(state.inner(), &key, &attachment_id).await? {
        AttachmentSlot::Existing(existing) => {
            return attach_existing_terminal(
                existing,
                &lookup,
                &attachment_id,
                scrollback_mode,
                &window,
                output,
                startup_command,
                attach_started,
            )
            .await;
        }
        AttachmentSlot::Reserved(reservation) => reservation,
    };

    let (scrollback_vt, scrollback_truncated) =
        prepare_attach_scrollback(&lookup, &attachment_id, scrollback_mode).await?;

    let spawn_started = Instant::now();
    let attachment = spawn_terminal_attachment(
        app,
        state.inner().clone(),
        lookup.clone(),
        key,
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
        scrollback_vt,
        scrollback_truncated,
    })
}

#[tauri::command]
pub async fn terminal_run_terminal(
    workspace: String,
    attachment_id: String,
    command: String,
) -> Result<TerminalRunResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    if !lookup.workspace.ready() {
        return Err(format!("workspace {workspace} is not ready"));
    }
    let session = session_for_command(&attachment_id, &command);
    let created = find_terminal_session(&lookup, &attachment_id)
        .await?
        .is_none();
    write_workspace_terminal_metadata(
        &workspace,
        &lookup,
        mutate_sessions(
            lookup.workspace.sessions(),
            lookup.workspace.last_working(),
            SessionMutation::Upsert(session.clone()),
        ),
    )
    .await?;
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

    Ok(TerminalRunResult { session, created })
}

pub(crate) async fn start_terminal_command(
    manager: &TerminalManager,
    workspace: &str,
    command: &str,
) -> Result<String, String> {
    let attachment_id = terminal_create_terminal(workspace.to_string())
        .await?
        .attachment_id;
    manager.set_startup_command(
        AttachmentKey {
            workspace: workspace.to_string(),
            name: attachment_id.clone(),
        },
        command.to_string(),
    );
    Ok(attachment_id)
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
    state: State<'_, TerminalManager>,
    workspace: String,
    attachment_id: String,
) -> Result<TerminalKillResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    let result = run_remote_command(
        &lookup,
        &run_terminal_user_command(&format!("zmx kill {}", shell_quote(&attachment_id))),
    )
    .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to kill terminal session",
            &result.stderr,
        ));
    }

    write_workspace_terminal_metadata(
        &workspace,
        &lookup,
        mutate_sessions(
            lookup.workspace.sessions(),
            lookup.workspace.last_working(),
            SessionMutation::Remove(&attachment_id),
        ),
    )
    .await?;

    let key = AttachmentKey {
        workspace,
        name: attachment_id,
    };
    if let Some(attachment) = state.remove_by_key(&key) {
        kill_local_attachment(&attachment)?;
    }

    Ok(TerminalKillResult { killed: true })
}

#[tauri::command]
pub async fn terminal_read_terminal(
    workspace: String,
    attachment_id: String,
) -> Result<TerminalReadResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    write_workspace_terminal_metadata(
        &workspace,
        &lookup,
        mutate_sessions(
            lookup.workspace.sessions(),
            lookup.workspace.last_working(),
            SessionMutation::MarkRead(&attachment_id),
        ),
    )
    .await?;
    let command = run_terminal_user_command(&format!(
        "if [ -x {observer_bin} ]; then {observer_bin} mark-read --session {attachment_id}; fi",
        observer_bin = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN),
        attachment_id = shell_quote(&attachment_id),
    ));
    let result = run_remote_command(&lookup, &command).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to mark terminal session as read",
            &result.stderr,
        ));
    }

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

fn spawn_terminal_attachment(
    app: AppHandle,
    manager: TerminalManager,
    lookup: WorkspaceLookup,
    key: AttachmentKey,
    output: Channel<Vec<u8>>,
    window_label: String,
) -> Result<Arc<Attachment>, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            cols: DEFAULT_TERMINAL_COLS,
            rows: DEFAULT_TERMINAL_ROWS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to create terminal pty: {error}"))?;
    let mut command = CommandBuilder::new("gcloud");
    command.args([
        format!("--account={}", lookup.account),
        format!("--project={}", lookup.gcloud_project),
        "compute".to_string(),
        "ssh".to_string(),
        lookup.workspace.name().to_string(),
        format!("--zone={}", lookup.workspace.zone()),
        "--ssh-flag=-tt".to_string(),
        format!(
            "--command={}",
            wrap_remote_shell_command(&terminal_attach_remote_command(&key.name))
        ),
    ]);

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
        output: Mutex::new(output),
        window_label: Mutex::new(window_label),
        connected: Mutex::new(false),
        connected_cv: Condvar::new(),
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
                    if let Ok(channel) = attachment.output.lock() {
                        if let Err(error) = channel.send(chunk) {
                            emit_terminal_error(
                                &attachment,
                                format!("failed to send terminal output: {error}"),
                            );
                            break;
                        }
                    } else {
                        emit_terminal_error(
                            &attachment,
                            "terminal output lock poisoned".to_string(),
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
    app: AppHandle,
    manager: TerminalManager,
    attachment: Arc<Attachment>,
    mut child: Box<dyn Child + Send + Sync>,
) {
    std::thread::spawn(move || {
        let status = child.wait();
        manager.remove_by_id(&attachment.id);

        match status {
            Ok(status) => {
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

fn emit_terminal_error_with_app(app: &AppHandle, attachment: &Attachment, message: String) {
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

fn current_window_label(attachment: &Attachment) -> Option<String> {
    attachment
        .window_label
        .lock()
        .ok()
        .map(|value| value.clone())
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
    scrollback_mode: AttachScrollbackMode,
    window: &WebviewWindow,
    output: Channel<Vec<u8>>,
    command: Option<String>,
    attach_started: Instant,
) -> Result<TerminalAttachResult, String> {
    let (scrollback_vt, scrollback_truncated) =
        prepare_attach_scrollback(lookup, name, scrollback_mode).await?;
    if let Ok(mut current_output) = existing.output.lock() {
        *current_output = output;
    }
    if let Ok(mut current_window) = existing.window_label.lock() {
        *current_window = window.label().to_string();
    }
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
        scrollback_vt,
        scrollback_truncated,
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

async fn list_terminals_in_workspace(
    lookup: &WorkspaceLookup,
) -> Result<Vec<WorkspaceSession>, String> {
    let mut sessions = lookup.workspace.sessions().to_vec();
    sort_workspace_sessions_oldest_to_newest(&mut sessions);
    Ok(sessions)
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

async fn load_scrollback(lookup: &WorkspaceLookup, name: &str) -> Result<(String, bool), String> {
    let result = run_remote_command(
        lookup,
        &run_terminal_user_command(&format!("zmx history {} --vt", shell_quote(name))),
    )
    .await?;
    if !result.success {
        if is_missing_terminal_session_error(&result.stderr) {
            return Ok((String::new(), false));
        }

        return Err(remote_command_error(
            "failed to load terminal scrollback",
            &result.stderr,
        ));
    }

    Ok(truncate_scrollback(result.stdout, MAX_SCROLLBACK_BYTES))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachScrollbackMode {
    Load,
    Skip,
}

fn attach_scrollback_mode(skip_scrollback: Option<bool>) -> AttachScrollbackMode {
    if skip_scrollback.unwrap_or(false) {
        AttachScrollbackMode::Skip
    } else {
        AttachScrollbackMode::Load
    }
}

async fn prepare_attach_scrollback(
    lookup: &WorkspaceLookup,
    attachment_id: &str,
    mode: AttachScrollbackMode,
) -> Result<(String, bool), String> {
    let started = Instant::now();
    match mode {
        AttachScrollbackMode::Skip => {
            log::info!(
                "terminal attach scrollback skipped workspace={} attachment_id={} elapsed_ms=0",
                lookup.workspace.name(),
                attachment_id
            );
            Ok((String::new(), false))
        }
        AttachScrollbackMode::Load => {
            log::info!(
                "terminal attach scrollback load start workspace={} attachment_id={}",
                lookup.workspace.name(),
                attachment_id
            );
            let result = load_scrollback(lookup, attachment_id).await;
            match &result {
                Ok((scrollback, truncated)) => {
                    log::info!(
                        "terminal attach scrollback load complete workspace={} attachment_id={} bytes={} truncated={} elapsed_ms={}",
                        lookup.workspace.name(),
                        attachment_id,
                        scrollback.len(),
                        truncated,
                        started.elapsed().as_millis()
                    );
                }
                Err(error) => {
                    log::warn!(
                        "terminal attach scrollback load failed workspace={} attachment_id={} elapsed_ms={} error={}",
                        lookup.workspace.name(),
                        attachment_id,
                        started.elapsed().as_millis(),
                        error
                    );
                }
            }
            result
        }
    }
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

async fn bootstrap_workspace(lookup: &WorkspaceLookup) -> Result<(), String> {
    let started = Instant::now();
    log::info!("bootstrapping workspace {}", lookup.workspace.name());
    let bootstrap = workspace_bootstrap(lookup)?;
    let bootstrap_signature = workspace_bootstrap_signature(lookup.workspace.name(), &bootstrap);
    let script = workspace_bootstrap_script(lookup, &bootstrap);
    let result = run_remote_command_with_stdin(
        lookup,
        &run_terminal_user_command("bash -se"),
        script.into_bytes(),
    )
    .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to bootstrap workspace",
            &result.stderr,
        ));
    }

    persist_workspace_bootstrap_state(lookup, &bootstrap_signature).await?;
    log::info!(
        "workspace {} bootstrap completed duration_ms={}",
        lookup.workspace.name(),
        started.elapsed().as_millis()
    );

    if lookup.workspace.is_template() {
        spawn_template_chrome_profile_sync(lookup.clone(), bootstrap);
    }
    Ok(())
}

fn bootstrap_template_workspace_task(
    workspace: &str,
) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
    let workspace = workspace.to_string();
    async move {
        for attempt in 0..TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
            let lookup = match workspaces::find_workspace(&workspace).await {
                Ok(lookup) => lookup,
                Err(error) => {
                    if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
                        return Err(error);
                    }
                    std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
                    continue;
                }
            };

            if !lookup.workspace.is_template() {
                return Err(format!(
                    "workspace {} is not a template workspace",
                    workspace
                ));
            }

            if lookup.workspace.status() != "RUNNING" {
                std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
                continue;
            }

            match bootstrap_workspace(&lookup).await {
                Ok(()) => {
                    workspaces::set_workspace_label(&workspace, "ready", "true").await?;
                    return Ok(());
                }
                Err(error) if should_retry_template_bootstrap(&error) => {
                    if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
                        return Err(error);
                    }
                    std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
                }
                Err(error) => return Err(error),
            }
        }

        Err(format!(
            "template workspace {} did not become ready for bootstrap after {} seconds",
            workspace,
            TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS * TEMPLATE_BOOTSTRAP_POLL_INTERVAL.as_secs() as usize
        ))
    }
}

pub(crate) fn start_template_bootstrap(workspace: String) {
    let inserted = TEMPLATE_BOOTSTRAP_IN_FLIGHT
        .lock()
        .map(|mut in_flight| in_flight.insert(workspace.clone()))
        .unwrap_or(false);
    if !inserted {
        return;
    }

    tauri::async_runtime::spawn(async move {
        if let Err(error) = workspaces::set_workspace_label(&workspace, "ready", "false").await {
            log::warn!(
                "failed to mark template workspace {} as not ready before bootstrap: {}",
                workspace,
                error
            );
        }

        let result = bootstrap_template_workspace_task(&workspace).await;
        if let Err(error) = result {
            log::warn!(
                "background template bootstrap failed for workspace {}: {}",
                workspace,
                error
            );
        } else {
            log::info!(
                "background template bootstrap completed for workspace {}",
                workspace
            );
        }

        if let Ok(mut in_flight) = TEMPLATE_BOOTSTRAP_IN_FLIGHT.lock() {
            in_flight.remove(&workspace);
        }
    });
}

pub(crate) fn start_workspace_ssh_readiness(workspace: String) {
    let inserted = WORKSPACE_SSH_READY_IN_FLIGHT
        .lock()
        .map(|mut in_flight| in_flight.insert(workspace.clone()))
        .unwrap_or(false);
    if !inserted {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let result = wait_until_workspace_ssh_ready(&workspace).await;
        if let Err(error) = result {
            log::warn!(
                "background ssh readiness check failed for workspace {}: {}",
                workspace,
                error
            );
        } else {
            log::info!("workspace {} is ssh-ready", workspace);
        }

        if let Ok(mut in_flight) = WORKSPACE_SSH_READY_IN_FLIGHT.lock() {
            in_flight.remove(&workspace);
        }
    });
}

pub(crate) async fn wait_for_template_bootstrap(workspace: &str) -> Result<(), String> {
    for _ in 0..TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
        let lookup = workspaces::find_workspace(workspace).await?;
        if !lookup.workspace.is_template() {
            return Err(format!("workspace {workspace} is not a template workspace"));
        }

        if lookup.workspace.ready() {
            return Ok(());
        }

        if !template_bootstrap_in_progress(workspace) {
            start_template_bootstrap(workspace.to_string());
        }

        std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
    }

    Err(format!(
        "template workspace {workspace} did not finish bootstrapping in time"
    ))
}

pub(crate) async fn wait_for_template_chrome_sync(workspace: &str) -> Result<(), String> {
    for _ in 0..TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
        if !template_chrome_sync_in_progress(workspace) {
            return Ok(());
        }
        std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
    }

    Err(format!(
        "chrome profile sync is still running for template workspace {workspace}"
    ))
}

fn should_retry_template_bootstrap(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "connection refused",
        "system is booting up",
        "not permitted to log in yet",
        "port 22",
        "broken pipe",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn template_bootstrap_in_progress(workspace: &str) -> bool {
    TEMPLATE_BOOTSTRAP_IN_FLIGHT
        .lock()
        .map(|in_flight| in_flight.contains(workspace))
        .unwrap_or(false)
}

async fn wait_until_workspace_ssh_ready(workspace: &str) -> Result<(), String> {
    let started = Instant::now();
    let mut logged_running = false;
    for attempt in 0..TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
        let lookup = match workspaces::find_workspace(workspace).await {
            Ok(lookup) => lookup,
            Err(error) => {
                if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
                continue;
            }
        };

        if lookup.workspace.status() != "RUNNING" {
            std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
            continue;
        }
        if !logged_running {
            log::info!(
                "workspace {} reached RUNNING; probing ssh readiness",
                workspace
            );
            logged_running = true;
        }

        let result = run_remote_command(&lookup, &run_terminal_user_command("true")).await;
        match result {
            Ok(result) if result.success => {
                log::info!(
                    "workspace {} ssh probe succeeded attempt={} elapsed_ms={}",
                    workspace,
                    attempt + 1,
                    started.elapsed().as_millis()
                );
                if !lookup.workspace.is_template() {
                    match bootstrap_workspace(&lookup).await {
                        Ok(()) => {}
                        Err(error) => {
                            if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS {
                                return Err(error);
                            }
                            if should_retry_template_bootstrap(&error) {
                                log::debug!(
                                    "workspace {} bootstrap retryable failure attempt={} elapsed_ms={} error={}",
                                    workspace,
                                    attempt + 1,
                                    started.elapsed().as_millis(),
                                    error
                                );
                                std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
                                continue;
                            }
                            return Err(error);
                        }
                    }
                }
                workspaces::set_workspace_label(workspace, "ready", "true").await?;
                log::info!(
                    "workspace {} marked ready elapsed_ms={}",
                    workspace,
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            Ok(result) => {
                let error = result.stderr.trim().to_string();
                if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(remote_command_error(
                        "failed to verify workspace ssh readiness",
                        &result.stderr,
                    ));
                }
                log::debug!(
                    "workspace {} ssh probe retryable failure attempt={} elapsed_ms={} error={}",
                    workspace,
                    attempt + 1,
                    started.elapsed().as_millis(),
                    error
                );
            }
            Err(error) => {
                if attempt + 1 == TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(error);
                }
                log::debug!(
                    "workspace {} ssh command retryable failure attempt={} elapsed_ms={} error={}",
                    workspace,
                    attempt + 1,
                    started.elapsed().as_millis(),
                    error
                );
            }
        }

        std::thread::sleep(TEMPLATE_BOOTSTRAP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace {workspace} did not become ssh-ready after {} seconds",
        TEMPLATE_BOOTSTRAP_POLL_ATTEMPTS * TEMPLATE_BOOTSTRAP_POLL_INTERVAL.as_secs() as usize
    ))
}

fn workspace_bootstrap(lookup: &WorkspaceLookup) -> Result<WorkspaceBootstrap, String> {
    let project_name = lookup.workspace.project().ok_or_else(|| {
        format!(
            "workspace {} is missing a project label",
            lookup.workspace.name()
        )
    })?;
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project = config.projects.get(project_name).ok_or_else(|| {
        format!(
            "project not found for workspace {}: {project_name}",
            lookup.workspace.name()
        )
    })?;

    if project.remote_url.trim().is_empty() {
        return Err(format!("project {project_name} is missing remote_url"));
    }

    let target_branch = if lookup.workspace.is_template() {
        project.target_branch.trim().to_string()
    } else {
        lookup
            .workspace
            .target_branch()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(project.target_branch.as_str())
            .trim()
            .to_string()
    };
    if target_branch.is_empty() {
        return Err(format!("project {project_name} is missing a target branch"));
    }

    let workspace_branch = if lookup.workspace.is_template() {
        None
    } else {
        let branch = lookup
            .workspace
            .branch_name()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if branch.is_empty() {
            return Err(format!(
                "workspace {} is missing branch metadata",
                lookup.workspace.name()
            ));
        }
        Some(branch)
    };

    let (chrome_user_data_dir, chrome_user_data_hash) = if lookup.workspace.is_template() {
        load_bootstrap_chrome_profile(&config.chrome.user_data_dir)
    } else {
        (String::new(), String::new())
    };

    Ok(WorkspaceBootstrap {
        remote_url: project.remote_url.clone(),
        target_branch,
        workspace_branch,
        gh_username: config.git.gh_username.clone(),
        gh_token: config.git.gh_token.clone(),
        chrome_user_data_dir,
        chrome_user_data_hash,
        codex_token: config.codex.token.clone(),
        claude_token: config.claude.token.clone(),
        git_user_name: config.git.user_name.clone(),
        git_user_email: config.git.user_email.clone(),
        env_files: load_bootstrap_env_files(project_name, project),
    })
}

fn workspace_bootstrap_script(lookup: &WorkspaceLookup, bootstrap: &WorkspaceBootstrap) -> String {
    let codex_auth_json = codex_auth_json(&bootstrap.codex_token);
    let codex_config_toml = codex_config_toml();
    let claude_settings_json = claude_settings_json();
    let claude_state_json = claude_state_json();
    let gh_hosts_yml = gh_hosts_yml(&bootstrap.gh_username, &bootstrap.gh_token);
    let bootstrap_signature = workspace_bootstrap_signature(lookup.workspace.name(), bootstrap);
    let observer_install = workspace_observer_install_script(lookup);
    let env_file_sync = if lookup.workspace.is_template() {
        workspace_env_file_sync_script(&bootstrap.env_files)
    } else {
        String::new()
    };
    let credentials_lines = [
        format!("export GH_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!("export GITHUB_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!(
            "export CLAUDE_CODE_OAUTH_TOKEN={}",
            shell_quote(&bootstrap.claude_token)
        ),
        "export GIT_ASKPASS=\"$HOME/.silo/git-askpass.sh\"".to_string(),
        "export GIT_TERMINAL_PROMPT=0".to_string(),
        "unset ANTHROPIC_API_KEY".to_string(),
        "unset ANTHROPIC_AUTH_TOKEN".to_string(),
        "unset ANTHROPIC_BASE_URL".to_string(),
        "unset CLAUDE_API_KEY".to_string(),
        "unset CLAUDE_CODE_USE_BEDROCK".to_string(),
        "unset CLAUDE_CODE_USE_VERTEX".to_string(),
        "unset AWS_BEARER_TOKEN_BEDROCK".to_string(),
        "unset VERTEX_REGION_CLAUDE_CODE".to_string(),
        format!(
            "if [[ -f {} ]]; then source {}; fi",
            shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE),
            shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE)
        ),
    ]
    .join("\n");
    let git_clone_target_branch = bootstrap_git_command(
        "clone --branch \"$TARGET_BRANCH\" \"$REMOTE_URL\" \"$WORKSPACE_DIR\"",
    );
    let git_fetch_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\"");
    let git_pull_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" pull --ff-only origin \"$TARGET_BRANCH\"");
    let branch_setup = if lookup.workspace.is_template() {
        format!(
            "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nelse\n  git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n  {git_fetch_target_branch}\n  git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" reset --hard \"origin/$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" clean -fd\nfi",
            git_clone_target_branch = git_clone_target_branch,
            git_fetch_target_branch = git_fetch_target_branch,
        )
    } else {
        format!(
            "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nfi\n\
git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n\
{git_fetch_target_branch}\n\
git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\"\n\
{git_pull_target_branch}\n\
if git -C \"$WORKSPACE_DIR\" show-ref --verify --quiet \"refs/heads/$WORKSPACE_BRANCH\"; then\n  git -C \"$WORKSPACE_DIR\" checkout \"$WORKSPACE_BRANCH\"\nelse\n  git -C \"$WORKSPACE_DIR\" checkout -b \"$WORKSPACE_BRANCH\" \"$TARGET_BRANCH\"\nfi",
            git_clone_target_branch = git_clone_target_branch,
            git_fetch_target_branch = git_fetch_target_branch,
            git_pull_target_branch = git_pull_target_branch,
        )
    };

    format!(
        "set -euo pipefail\n\
WORKSPACE_DIR={workspace_dir}\n\
REMOTE_URL={remote_url}\n\
TARGET_BRANCH={target_branch}\n\
WORKSPACE_BRANCH={workspace_branch}\n\
GIT_USER_NAME={git_user_name}\n\
GIT_USER_EMAIL={git_user_email}\n\
mkdir -p \"$HOME/.silo\"\n\
chmod 700 \"$HOME/.silo\"\n\
LOCK_DIR={lock_dir}\n\
ASKPASS_PATH=\"$HOME/.silo/git-askpass.sh\"\n\
cleanup() {{\n\
  rm -rf \"$LOCK_DIR\"\n\
}}\n\
for _ in $(seq 1 60); do\n\
  if mkdir \"$LOCK_DIR\" 2>/dev/null; then\n\
    trap cleanup EXIT\n\
    break\n\
  fi\n\
  sleep 1\n\
done\n\
if [ ! -d \"$LOCK_DIR\" ]; then\n\
  echo 'timed out waiting for workspace bootstrap lock' >&2\n\
  exit 1\n\
fi\n\
BOOT_ID=\"$(cat /proc/sys/kernel/random/boot_id)\"\n\
STATE_PATH={state_path}\n\
SIGNATURE={signature}\n\
if [ -f \"$STATE_PATH\" ]; then\n\
  CURRENT_BOOT_ID=\"$(sed -n '1p' \"$STATE_PATH\")\"\n\
  CURRENT_SIGNATURE=\"$(sed -n '2,$p' \"$STATE_PATH\")\"\n\
  if [ \"$CURRENT_BOOT_ID\" = \"$BOOT_ID\" ] && [ \"$CURRENT_SIGNATURE\" = \"$SIGNATURE\" ]; then\n\
    exit 0\n\
  fi\n\
fi\n\
cat > \"$ASKPASS_PATH\" <<'EOF_GIT_ASKPASS'\n\
#!/bin/sh\n\
case \"$1\" in\n\
  *Username*) printf '%s\\n' 'x-access-token' ;;\n\
  *Password*) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
  *) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
esac\n\
EOF_GIT_ASKPASS\n\
chmod 700 \"$ASKPASS_PATH\"\n\
cat > {credentials_path} <<'EOF'\n\
{credentials_lines}\n\
EOF\n\
chmod 600 {credentials_path}\n\
. {credentials_path}\n\
for rc in \"$HOME/.zshrc\" \"$HOME/.bashrc\"; do\n\
  touch \"$rc\"\n\
  if ! grep -Fqx '[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"' \"$rc\"; then\n\
    printf '\\n[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"\\n' >> \"$rc\"\n\
  fi\n\
done\n\
mkdir -p \"$HOME/.config/gh\"\n\
printf '%s\\n' {gh_hosts_yml} > \"$HOME/.config/gh/hosts.yml\"\n\
chmod 700 \"$HOME/.config\" \"$HOME/.config/gh\"\n\
chmod 600 \"$HOME/.config/gh/hosts.yml\"\n\
mkdir -p \"$HOME/.codex\" \"$HOME/.claude\"\n\
printf '%s\\n' {codex_auth_json} > \"$HOME/.codex/auth.json\"\n\
printf '%s\\n' {codex_config_toml} > \"$HOME/.codex/config.toml\"\n\
printf '%s\\n' {claude_settings_json} > \"$HOME/.claude/settings.json\"\n\
printf '%s\\n' {claude_state_json} > \"$HOME/.claude.json\"\n\
chmod 700 \"$HOME/.codex\" \"$HOME/.claude\"\n\
chmod 600 \"$HOME/.codex/auth.json\" \"$HOME/.codex/config.toml\" \"$HOME/.claude/settings.json\" \"$HOME/.claude.json\"\n\
rm -f \"$HOME/.gitconfig.lock\"\n\
if [ -n \"$GIT_USER_NAME\" ] && [ \"$(git config --global --get user.name || true)\" != \"$GIT_USER_NAME\" ]; then\n\
  git config --global user.name \"$GIT_USER_NAME\"\n\
fi\n\
if [ -n \"$GIT_USER_EMAIL\" ] && [ \"$(git config --global --get user.email || true)\" != \"$GIT_USER_EMAIL\" ]; then\n\
  git config --global user.email \"$GIT_USER_EMAIL\"\n\
fi\n\
if ! git config --global --get-all safe.directory 2>/dev/null | grep -Fxq \"$WORKSPACE_DIR\"; then\n\
  git config --global --add safe.directory \"$WORKSPACE_DIR\"\n\
fi\n\
{branch_setup}\n\
{env_file_sync}\n\
{observer_install}",
        workspace_dir = shell_quote(TERMINAL_WORKSPACE_DIR),
        remote_url = shell_quote(&bootstrap.remote_url),
        target_branch = shell_quote(&bootstrap.target_branch),
        workspace_branch = shell_quote(bootstrap.workspace_branch.as_deref().unwrap_or("")),
        git_user_name = shell_quote(&bootstrap.git_user_name),
        git_user_email = shell_quote(&bootstrap.git_user_email),
        lock_dir = shell_quote(REMOTE_BOOTSTRAP_LOCK_DIR),
        state_path = shell_quote(REMOTE_BOOTSTRAP_STATE_FILE),
        signature = shell_quote(&bootstrap_signature),
        credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE),
        credentials_lines = credentials_lines,
        gh_hosts_yml = shell_quote(&gh_hosts_yml),
        codex_auth_json = shell_quote(&codex_auth_json),
        codex_config_toml = shell_quote(&codex_config_toml),
        claude_settings_json = shell_quote(&claude_settings_json),
        claude_state_json = shell_quote(&claude_state_json),
        branch_setup = branch_setup,
        env_file_sync = env_file_sync,
        observer_install = observer_install,
    )
}

fn bootstrap_git_command(command: &str) -> String {
    format!(
        "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git {command}"
    )
}

fn workspace_bootstrap_signature(workspace_name: &str, bootstrap: &WorkspaceBootstrap) -> String {
    format!(
        "version={}\nworkspace={}\nremote_url={}\ntarget_branch={}\nworkspace_branch={}\ngh_username={}\ngh_token={}\nchrome_user_data_dir={}\nchrome_user_data_hash={}\ncodex_token={}\nclaude_token={}\ngit_user_name={}\ngit_user_email={}\nenv_files={}\nobserver_sources={}",
        WORKSPACE_BOOTSTRAP_VERSION,
        workspace_name,
        bootstrap.remote_url,
        bootstrap.target_branch,
        bootstrap.workspace_branch.as_deref().unwrap_or(""),
        bootstrap.gh_username,
        bootstrap.gh_token,
        bootstrap.chrome_user_data_dir,
        bootstrap.chrome_user_data_hash,
        bootstrap.codex_token,
        bootstrap.claude_token,
        bootstrap.git_user_name,
        bootstrap.git_user_email,
        bootstrap_env_files_signature(&bootstrap.env_files),
        workspace_observer_source_signature(),
    )
}

fn load_bootstrap_chrome_profile(configured_path: &str) -> (String, String) {
    let trimmed = configured_path.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }

    let source_dir = PathBuf::from(trimmed);
    if !source_dir.is_dir() {
        log::warn!(
            "skipping missing chrome user data directory configured for bootstrap: {}",
            source_dir.display()
        );
        return (String::new(), String::new());
    }

    let sync_paths = chrome_profile_sync_paths(&source_dir);
    if sync_paths.is_empty() {
        log::warn!(
            "skipping chrome profile bootstrap because no syncable profile data was found in {}",
            source_dir.display()
        );
        return (String::new(), String::new());
    }

    let hash = match hash_chrome_profile_dir(&source_dir, &sync_paths) {
        Ok(hash) => hash,
        Err(error) => {
            log::warn!(
                "skipping chrome profile bootstrap because hashing failed for {}: {}",
                source_dir.display(),
                error
            );
            return (String::new(), String::new());
        }
    };

    (source_dir.to_string_lossy().into_owned(), hash)
}

fn load_bootstrap_env_files(
    project_name: &str,
    project: &crate::config::ProjectConfig,
) -> Vec<BootstrapEnvFile> {
    let project_root = Path::new(&project.path);
    let mut env_files = Vec::new();
    let mut seen = HashSet::new();

    for configured_path in &project.env_files {
        let Some(relative_path) = normalize_workspace_relative_path(configured_path) else {
            log::warn!(
                "skipping invalid env file path for project {}: {}",
                project_name,
                configured_path
            );
            continue;
        };

        if !seen.insert(relative_path.clone()) {
            continue;
        }

        let local_path = project_root.join(Path::new(&relative_path));
        let Ok(contents) = fs::read(&local_path) else {
            log::warn!(
                "skipping missing or unreadable env file for project {}: {}",
                project_name,
                local_path.display()
            );
            continue;
        };

        let contents_sha256 = hex_sha256(&contents);
        env_files.push(BootstrapEnvFile {
            relative_path,
            contents_base64: BASE64_STANDARD.encode(contents),
            contents_sha256,
        });
    }

    env_files
}

fn normalize_workspace_relative_path(path: &str) -> Option<String> {
    let mut normalized = String::new();

    for component in Path::new(path).components() {
        let Component::Normal(value) = component else {
            return None;
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(value.to_str()?);
    }

    (!normalized.is_empty()).then_some(normalized)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn bootstrap_env_files_signature(env_files: &[BootstrapEnvFile]) -> String {
    env_files
        .iter()
        .map(|env_file| format!("{}:{}", env_file.relative_path, env_file.contents_sha256))
        .collect::<Vec<_>>()
        .join(",")
}

fn chrome_profile_sync_paths(source_dir: &Path) -> Vec<PathBuf> {
    CHROME_PROFILE_SYNC_PATHS
        .iter()
        .map(Path::new)
        .filter(|relative_path| source_dir.join(relative_path).exists())
        .map(PathBuf::from)
        .collect()
}

fn hash_chrome_profile_dir(source_dir: &Path, sync_paths: &[PathBuf]) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for sync_path in sync_paths {
        hash_chrome_profile_entry(source_dir, &source_dir.join(sync_path), &mut hasher)?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_chrome_profile_entry(root: &Path, path: &Path, hasher: &mut Sha256) -> Result<(), String> {
    let relative_path = path.strip_prefix(root).map_err(|error| {
        format!(
            "failed to normalize chrome path {}: {error}",
            path.display()
        )
    })?;

    if !relative_path.as_os_str().is_empty() && should_skip_chrome_profile_entry(relative_path) {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to read chrome path {}: {error}", path.display()))?;
    let relative = relative_path.to_string_lossy();

    if metadata.file_type().is_symlink() {
        hasher.update(b"L");
        hasher.update(relative.as_bytes());
        let link_target = fs::read_link(path).map_err(|error| {
            format!("failed to read chrome symlink {}: {error}", path.display())
        })?;
        hasher.update(link_target.to_string_lossy().as_bytes());
        return Ok(());
    }

    if metadata.is_file() {
        hasher.update(b"F");
        hasher.update(relative.as_bytes());
        let mut file = fs::File::open(path)
            .map_err(|error| format!("failed to open chrome file {}: {error}", path.display()))?;
        let mut buffer = [0u8; 8192];
        loop {
            let count = file.read(&mut buffer).map_err(|error| {
                format!("failed to read chrome file {}: {error}", path.display())
            })?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        return Ok(());
    }

    if metadata.is_dir() {
        hasher.update(b"D");
        hasher.update(relative.as_bytes());
        let mut entries: Vec<PathBuf> = fs::read_dir(path)
            .map_err(|error| {
                format!(
                    "failed to list chrome directory {}: {error}",
                    path.display()
                )
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        entries.sort();

        for entry in entries {
            hash_chrome_profile_entry(root, &entry, hasher)?;
        }
    }

    Ok(())
}

fn should_skip_chrome_profile_entry(relative_path: &Path) -> bool {
    relative_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|name| {
            matches!(
                name,
                "SingletonCookie" | "SingletonLock" | "SingletonSocket" | "DevToolsActivePort"
            )
        })
        .unwrap_or(false)
}

fn workspace_env_file_sync_script(env_files: &[BootstrapEnvFile]) -> String {
    if env_files.is_empty() {
        return String::new();
    }

    let mut script = String::from("# sync project env files into the template workspace\n");

    for (index, env_file) in env_files.iter().enumerate() {
        if let Some((parent_dir, _)) = env_file.relative_path.rsplit_once('/') {
            script.push_str(&format!(
                "mkdir -p {}\n",
                shell_quote(&format!("{TERMINAL_WORKSPACE_DIR}/{parent_dir}"))
            ));
        }

        let target_path = format!("{TERMINAL_WORKSPACE_DIR}/{}", env_file.relative_path);
        script.push_str(&format!(
            "cat <<'EOF_ENV_{index}' | base64 --decode > {target_path}\n{contents}\nEOF_ENV_{index}\nchmod 600 {target_path}\n",
            target_path = shell_quote(&target_path),
            contents = env_file.contents_base64,
        ));
    }

    script
}

fn workspace_observer_install_script(lookup: &WorkspaceLookup) -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN);
    let pidfile = shell_quote(REMOTE_WORKSPACE_OBSERVER_PIDFILE);
    let shell_path = shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE);
    let encoded_binary = BASE64_STANDARD.encode(WORKSPACE_OBSERVER_BIN_BYTES);
    let shell_script = workspace_observer_shell_script();
    let encoded_shell = BASE64_STANDARD.encode(shell_script.as_bytes());

    let mut script =
        "install -d -m 0700 \"$HOME/.silo\" \"$HOME/.silo/bin\" \"$HOME/.silo/workspace-observer\"\n"
            .to_string();
    script.push_str(&format!(
        "cat <<'EOF_OBSERVER_BIN' | base64 --decode > {bin_path}\n{encoded_binary}\nEOF_OBSERVER_BIN\n",
    ));
    script.push_str(&format!("chmod 0755 {bin_path}\n"));
    script.push_str(&format!(
        "cat <<'EOF_OBSERVER_SHELL' | base64 --decode > {shell_path}\n{encoded_shell}\nEOF_OBSERVER_SHELL\n",
    ));
    script.push_str(&format!("chmod 0755 {shell_path}\n"));
    script.push_str(&format!(
        "if [ -f {pidfile} ]; then kill \"$(cat {pidfile})\" 2>/dev/null || true; rm -f {pidfile}; fi\n",
    ));
    script.push_str(&format!(
        "nohup {bin_path} daemon --instance {instance} --project {project} --zone {zone} >/dev/null 2>&1 < /dev/null &\n",
        instance = shell_quote(lookup.workspace.name()),
        project = shell_quote(&lookup.gcloud_project),
        zone = shell_quote(lookup.workspace.zone()),
    ));
    script
}

fn workspace_observer_source_signature() -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_OBSERVER_BIN_BYTES);
    hasher.update(workspace_observer_shell_script().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn workspace_observer_shell_script() -> String {
    format!(
        "export SILO_WORKSPACE_OBSERVER_BIN={observer_bin}\n\
_silo_observer_emit() {{\n\
  [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
  [ -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ] || return 0\n\
  SILO_OBSERVER_HOOK=1 \"$SILO_WORKSPACE_OBSERVER_BIN\" emit \"$@\" >/dev/null 2>&1 || true\n\
}}\n\
_silo_observer_wrap_assistant() {{\n\
  local provider=\"$1\"\n\
  shift\n\
  if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ]; then\n\
    command \"$@\"\n\
    return\n\
  fi\n\
  command \"$SILO_WORKSPACE_OBSERVER_BIN\" assistant-proxy --provider \"$provider\" -- \"$@\"\n\
}}\n\
codex() {{\n\
  _silo_observer_wrap_assistant codex codex \"$@\"\n\
}}\n\
claude() {{\n\
  _silo_observer_wrap_assistant claude claude --dangerously-skip-permissions \"$@\"\n\
}}\n\
cc() {{\n\
  claude \"$@\"\n\
}}\n\
case $- in\n\
  *i*) ;;\n\
  *) return 0 2>/dev/null || exit 0 ;;\n\
esac\n\
if [ -n \"${{ZSH_VERSION:-}}\" ]; then\n\
  autoload -Uz add-zsh-hook\n\
  typeset -g SILO_OBSERVER_LAST_COMMAND=\"${{SILO_OBSERVER_LAST_COMMAND:-}}\"\n\
  _silo_observer_preexec() {{\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && return 0\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    SILO_OBSERVER_LAST_COMMAND=\"$1\"\n\
    _silo_observer_emit --kind shell_command_started --session \"$ZMX_SESSION\" --command \"$1\"\n\
  }}\n\
  _silo_observer_precmd() {{\n\
    local exit_code=$?\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && return $exit_code\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return $exit_code\n\
    if [ -n \"${{SILO_OBSERVER_LAST_COMMAND:-}}\" ]; then\n\
      _silo_observer_emit --kind shell_command_finished --session \"$ZMX_SESSION\"\n\
      SILO_OBSERVER_LAST_COMMAND=\"\"\n\
    fi\n\
    return $exit_code\n\
  }}\n\
  case \" ${{preexec_functions[*]:-}} \" in\n\
    *\" _silo_observer_preexec \"*) ;;\n\
    *) add-zsh-hook preexec _silo_observer_preexec ;;\n\
  esac\n\
  case \" ${{precmd_functions[*]:-}} \" in\n\
    *\" _silo_observer_precmd \"*) ;;\n\
    *) add-zsh-hook precmd _silo_observer_precmd ;;\n\
  esac\n\
fi\n",
        observer_bin = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN),
    )
}

fn codex_auth_json(token: &str) -> String {
    let last_refresh = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": serde_json::Value::Null,
        "tokens": {
            "id_token": token,
            "access_token": token,
            "refresh_token": token,
            "account_id": ""
        },
        "last_refresh": last_refresh
    })
    .to_string()
}

fn codex_config_toml() -> String {
    r#"personality = "pragmatic"
model = "gpt-5.4"
model_reasoning_effort = "high"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[projects."/home/silo/workspace"]
trust_level = "trusted"

[notice]
hide_full_access_warning = true
"#
    .to_string()
}

fn claude_settings_json() -> String {
    json!({
        "model": "opus",
        "alwaysThinkingEnabled": true,
        "effortLevel": "high",
        "skipDangerousModePermissionPrompt": true
    })
    .to_string()
}

fn claude_state_json() -> String {
    json!({
        "installMethod": "native",
        "autoUpdates": false,
        "hasCompletedOnboarding": true,
        "projects": {
            TERMINAL_WORKSPACE_DIR: {
                "allowedTools": [],
                "mcpContextUris": [],
                "mcpServers": {},
                "enabledMcpjsonServers": [],
                "disabledMcpjsonServers": [],
                "hasTrustDialogAccepted": true,
                "projectOnboardingSeenCount": 1,
                "hasCompletedProjectOnboarding": true,
                "hasClaudeMdExternalIncludesApproved": false,
                "hasClaudeMdExternalIncludesWarningShown": false
            }
        }
    })
    .to_string()
}

fn gh_hosts_yml(username: &str, token: &str) -> String {
    format!(
        "github.com:\n    user: {username}\n    oauth_token: {token}\n    git_protocol: https\n"
    )
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

fn terminal_shell_command(command: &str) -> String {
    format!(
        "if [ -f {credentials_path} ]; then source {credentials_path}; fi; cd {workspace_dir}; {command}",
        credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE),
        workspace_dir = shell_quote(TERMINAL_WORKSPACE_DIR),
        command = command,
    )
}

async fn sync_template_chrome_profile(
    lookup: &WorkspaceLookup,
    bootstrap: &WorkspaceBootstrap,
) -> Result<(), String> {
    if bootstrap.chrome_user_data_dir.is_empty() {
        return Ok(());
    }

    let source_dir = PathBuf::from(&bootstrap.chrome_user_data_dir);
    if !source_dir.is_dir() {
        log::warn!(
            "skipping missing chrome user data directory during template bootstrap: {}",
            source_dir.display()
        );
        return Ok(());
    }

    let sync_paths = chrome_profile_sync_paths(&source_dir);
    if sync_paths.is_empty() {
        log::warn!(
            "skipping empty chrome profile sync set for template workspace {} from {}",
            lookup.workspace.name(),
            source_dir.display()
        );
        return Ok(());
    }

    let remote_command = chrome_profile_remote_sync_command();
    let result =
        run_remote_command_with_tar_stream(lookup, &remote_command, &source_dir, sync_paths)
            .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to sync chrome profile into template workspace",
            &result.stderr,
        ));
    }

    Ok(())
}

async fn persist_workspace_bootstrap_state(
    lookup: &WorkspaceLookup,
    signature: &str,
) -> Result<(), String> {
    let command = persist_workspace_bootstrap_state_command(signature);
    let result = run_remote_command(lookup, &run_terminal_user_command(&command)).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to persist workspace bootstrap state",
            &result.stderr,
        ));
    }

    Ok(())
}

fn persist_workspace_bootstrap_state_command(signature: &str) -> String {
    let state_path = shell_quote(REMOTE_BOOTSTRAP_STATE_FILE);
    let signature_base64 = shell_quote(&BASE64_STANDARD.encode(signature));
    format!(
        "mkdir -p \"$HOME/.silo\" && BOOT_ID=\"$(cat /proc/sys/kernel/random/boot_id)\" && {{ printf '%s\\n' \"$BOOT_ID\"; printf %s {signature_base64} | base64 --decode; printf '\\n'; }} > {state_path} && chmod 600 {state_path}",
    )
}

fn spawn_template_chrome_profile_sync(lookup: WorkspaceLookup, bootstrap: WorkspaceBootstrap) {
    if bootstrap.chrome_user_data_dir.is_empty() || bootstrap.chrome_user_data_hash.is_empty() {
        return;
    }

    let workspace_name = lookup.workspace.name().to_string();
    let inserted = TEMPLATE_CHROME_SYNC_IN_FLIGHT
        .lock()
        .map(|mut in_flight| in_flight.insert(workspace_name.clone()))
        .unwrap_or(false);
    if !inserted {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let result = sync_template_chrome_profile(&lookup, &bootstrap).await;
        if let Err(error) = result {
            log::warn!(
                "background chrome profile sync failed for template workspace {}: {}",
                workspace_name,
                error
            );
        } else {
            log::info!(
                "background chrome profile sync completed for template workspace {}",
                workspace_name
            );
        }

        if let Ok(mut in_flight) = TEMPLATE_CHROME_SYNC_IN_FLIGHT.lock() {
            in_flight.remove(&workspace_name);
        }
    });
}

fn template_chrome_sync_in_progress(workspace: &str) -> bool {
    TEMPLATE_CHROME_SYNC_IN_FLIGHT
        .lock()
        .map(|in_flight| in_flight.contains(workspace))
        .unwrap_or(false)
}

fn chrome_profile_remote_sync_command() -> String {
    let remote_dir = shell_quote(REMOTE_CHROME_USER_DATA_DIR);
    run_terminal_user_command(&format!(
        "mkdir -p \"$HOME/.config\" && rm -rf {remote_dir} && mkdir -p {remote_dir} && tar -xzf - -C {remote_dir} --strip-components=1 && chmod 700 \"$HOME/.config\" {remote_dir} && chmod -R u=rwX,go= {remote_dir}",
    ))
}

pub(crate) async fn run_remote_command(
    lookup: &WorkspaceLookup,
    remote_command: &str,
) -> Result<CommandResult, String> {
    run_gcloud_ssh_command(lookup, Some(remote_command.to_string()), None).await
}

async fn run_remote_command_with_stdin(
    lookup: &WorkspaceLookup,
    remote_command: &str,
    stdin_bytes: Vec<u8>,
) -> Result<CommandResult, String> {
    run_gcloud_ssh_command(lookup, Some(remote_command.to_string()), Some(stdin_bytes)).await
}

async fn run_gcloud_ssh_command(
    lookup: &WorkspaceLookup,
    remote_command: Option<String>,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<CommandResult, String> {
    let account = lookup.account.clone();
    let project = lookup.gcloud_project.clone();
    let workspace = lookup.workspace.name().to_string();
    let zone = lookup.workspace.zone().to_string();

    tauri::async_runtime::spawn_blocking(move || {
        let mut command =
            build_gcloud_ssh_command(&account, &project, &workspace, &zone, remote_command);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin_bytes.is_some() {
            command.stdin(Stdio::piped());
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to execute gcloud ssh: {error}"))?;

        if let Some(stdin_bytes) = stdin_bytes {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(error) = stdin.write_all(&stdin_bytes) {
                    drop(stdin);
                    let output = child.wait_with_output().map_err(|wait_error| {
                        format!("failed to read gcloud ssh output: {wait_error}")
                    })?;
                    return Ok(command_result_with_stdin_write_error(output, &error));
                }
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to read gcloud ssh output: {error}"))?;

        Ok(command_result_from_output(output))
    })
    .await
    .map_err(|error| format!("gcloud ssh task failed: {error}"))?
}

fn command_result_from_output(output: Output) -> CommandResult {
    CommandResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn command_result_with_stdin_write_error(output: Output, error: &std::io::Error) -> CommandResult {
    let mut result = command_result_from_output(output);
    let write_error = format!("failed to write gcloud ssh stdin: {error}");
    result.success = false;
    result.stderr = if result.stderr.trim().is_empty() {
        write_error
    } else {
        format!("{}\n{write_error}", result.stderr.trim_end())
    };
    result
}

async fn run_remote_command_with_tar_stream(
    lookup: &WorkspaceLookup,
    remote_command: &str,
    source_dir: &Path,
    sync_paths: Vec<PathBuf>,
) -> Result<CommandResult, String> {
    let account = lookup.account.clone();
    let project = lookup.gcloud_project.clone();
    let workspace = lookup.workspace.name().to_string();
    let zone = lookup.workspace.zone().to_string();
    let remote_command = remote_command.to_string();
    let source_dir = source_dir.to_path_buf();
    let sync_paths = sync_paths;

    tauri::async_runtime::spawn_blocking(move || {
        let parent = source_dir.parent().ok_or_else(|| {
            format!(
                "chrome user data directory has no parent: {}",
                source_dir.display()
            )
        })?;
        let file_name = source_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                format!(
                    "chrome user data directory has no terminal path segment: {}",
                    source_dir.display()
                )
            })?;

        let mut tar = Command::new("tar");
        tar.arg("-C").arg(parent);
        tar.arg("-czf").arg("-");
        for sync_path in &sync_paths {
            tar.arg(Path::new(file_name).join(sync_path));
        }
        tar.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut tar_child = tar
            .spawn()
            .map_err(|error| format!("failed to start tar for chrome profile sync: {error}"))?;

        let mut command =
            build_gcloud_ssh_command(&account, &project, &workspace, &zone, Some(remote_command));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut gcloud = command
            .spawn()
            .map_err(|error| format!("failed to execute gcloud ssh: {error}"))?;

        let mut tar_stdout = tar_child
            .stdout
            .take()
            .ok_or_else(|| "failed to capture tar stdout".to_string())?;
        let mut gcloud_stdin = gcloud
            .stdin
            .take()
            .ok_or_else(|| "failed to open gcloud ssh stdin".to_string())?;
        std::io::copy(&mut tar_stdout, &mut gcloud_stdin)
            .map_err(|error| format!("failed to stream chrome profile archive: {error}"))?;
        drop(gcloud_stdin);

        let tar_output = tar_child
            .wait_with_output()
            .map_err(|error| format!("failed to read tar output: {error}"))?;
        if !tar_output.status.success() {
            return Err(format!(
                "failed to create chrome profile archive: {}",
                String::from_utf8_lossy(&tar_output.stderr).trim()
            ));
        }

        let output = gcloud
            .wait_with_output()
            .map_err(|error| format!("failed to read gcloud ssh output: {error}"))?;
        Ok(CommandResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    })
    .await
    .map_err(|error| format!("gcloud ssh task failed: {error}"))?
}

fn build_gcloud_ssh_command(
    account: &str,
    project: &str,
    workspace: &str,
    zone: &str,
    remote_command: Option<String>,
) -> Command {
    let mut command = Command::new("gcloud");
    command.arg(format!("--account={account}"));
    command.arg(format!("--project={project}"));
    command.arg("compute");
    command.arg("ssh");
    command.arg(workspace);
    command.arg(format!("--zone={zone}"));

    if let Some(remote_command) = remote_command {
        command.arg(format!(
            "--command={}",
            wrap_remote_shell_command(&remote_command)
        ));
    }

    command
}

fn wrap_remote_shell_command(command: &str) -> String {
    command.to_string()
}

fn run_terminal_user_command(command: &str) -> String {
    format!("sudo -iu {TERMINAL_USER} bash -lc {}", shell_quote(command))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn workspace_shell_command(command: &str) -> String {
    workspace_shell_command_with_prelude(None, command)
}

pub(crate) fn workspace_shell_command_with_credentials(command: &str) -> String {
    let credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE);
    workspace_shell_command_with_prelude(
        Some(&format!(
            "if [ -f {credentials_path} ]; then\n. {credentials_path}\nfi"
        )),
        command,
    )
}

fn workspace_shell_command_with_prelude(prelude: Option<&str>, command: &str) -> String {
    let script = workspace_shell_script(prelude, command);
    let encoded = BASE64_STANDARD.encode(script);
    run_terminal_user_command(&format!(
        "printf %s {} | base64 --decode | bash",
        shell_quote(&encoded)
    ))
}

fn workspace_shell_script(prelude: Option<&str>, command: &str) -> String {
    format!(
        "set -euo pipefail\nexport LC_ALL=C\nexport LANG=C\n{}\ncd {}\n{}",
        prelude.unwrap_or_default(),
        shell_quote(TERMINAL_WORKSPACE_DIR),
        command
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn remote_command_error(prefix: &str, stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {trimmed}")
    }
}

fn is_missing_terminal_session_error(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("session not found")
        || lower.contains("no such session")
        || lower.contains("unknown session")
}

fn pending_terminal_session(attachment_id: &str) -> WorkspaceSession {
    WorkspaceSession {
        kind: "terminal".to_string(),
        name: "shell".to_string(),
        attachment_id: attachment_id.to_string(),
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

fn current_unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn sort_workspace_sessions_oldest_to_newest(sessions: &mut [WorkspaceSession]) {
    sessions.sort_by(|left, right| {
        terminal_name_timestamp(&left.attachment_id)
            .cmp(&terminal_name_timestamp(&right.attachment_id))
            .then_with(|| left.attachment_id.cmp(&right.attachment_id))
    });
}

fn terminal_name_timestamp(name: &str) -> Option<u128> {
    name.strip_prefix("terminal-")?.parse::<u128>().ok()
}

fn truncate_scrollback(mut scrollback: String, max_bytes: usize) -> (String, bool) {
    if scrollback.len() <= max_bytes {
        return (scrollback, false);
    }

    let start = scrollback
        .char_indices()
        .find(|(index, _)| *index >= scrollback.len() - max_bytes)
        .map(|(index, _)| index)
        .unwrap_or(0);
    scrollback.drain(..start);
    (scrollback, true)
}

enum SessionMutation<'a> {
    Upsert(WorkspaceSession),
    Remove(&'a str),
    MarkRead(&'a str),
}

struct WorkspaceTerminalMetadata {
    sessions: Vec<WorkspaceSession>,
    unread: bool,
    working: Option<bool>,
    last_active: String,
    last_working: Option<String>,
}

fn mutate_sessions(
    current_sessions: &[WorkspaceSession],
    current_last_working: Option<&str>,
    mutation: SessionMutation<'_>,
) -> WorkspaceTerminalMetadata {
    let mut sessions = current_sessions.to_vec();
    match mutation {
        SessionMutation::Upsert(session) => {
            if let Some(existing) = sessions
                .iter_mut()
                .find(|existing| existing.attachment_id == session.attachment_id)
            {
                *existing = session;
            } else {
                sessions.push(session);
            }
        }
        SessionMutation::Remove(attachment_id) => {
            sessions.retain(|session| session.attachment_id != attachment_id);
        }
        SessionMutation::MarkRead(attachment_id) => {
            if let Some(session) = sessions
                .iter_mut()
                .find(|session| session.attachment_id == attachment_id)
            {
                if session.unread.is_some() {
                    session.unread = Some(false);
                }
            }
        }
    }
    sort_workspace_sessions_oldest_to_newest(&mut sessions);
    let now = current_rfc3339_timestamp();
    let working = sessions
        .iter()
        .any(|session| session.working == Some(true))
        .then_some(true);
    WorkspaceTerminalMetadata {
        unread: sessions.iter().any(|session| session.unread == Some(true)),
        working,
        sessions,
        last_active: now.clone(),
        last_working: if working.is_some() {
            Some(now)
        } else {
            current_last_working.map(str::to_string)
        },
    }
}

async fn write_workspace_terminal_metadata(
    workspace: &str,
    lookup: &WorkspaceLookup,
    metadata: WorkspaceTerminalMetadata,
) -> Result<(), String> {
    let sessions_json =
        serde_json::to_string(&metadata.sessions).map_err(|error| error.to_string())?;
    let unread = if metadata.unread { "true" } else { "false" };
    let working = if metadata.working.unwrap_or(false) {
        "true"
    } else {
        "false"
    };
    let last_active = metadata.last_active.as_str();
    let mut entries = vec![
        ("sessions", sessions_json.as_str()),
        ("unread", unread),
        ("working", working),
        ("last_active", last_active),
    ];
    if let Some(last_working) = metadata.last_working.as_deref() {
        entries.push(("last_working", last_working));
    }
    workspaces::set_workspace_metadata_entries(workspace, &entries).await?;

    log::trace!(
        "updated terminal metadata for workspace {} sessions={} unread={} working={}",
        lookup.workspace.name(),
        metadata.sessions.len(),
        unread,
        working
    );
    Ok(())
}

fn session_for_command(attachment_id: &str, command: &str) -> WorkspaceSession {
    let name = sanitize_session_display_name(command);
    let assistant_capable = assistant_capable_command(&name);
    WorkspaceSession {
        kind: "terminal".to_string(),
        name,
        attachment_id: attachment_id.to_string(),
        working: assistant_capable.then_some(false),
        unread: assistant_capable.then_some(false),
    }
}

fn sanitize_session_display_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "shell".to_string();
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

fn current_rfc3339_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
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
    use crate::config::ProjectConfig;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::process::{ExitStatus, Output};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("ab'cd"), "'ab'\"'\"'cd'");
    }

    #[test]
    fn wrap_remote_shell_command_passes_through_command() {
        assert_eq!(wrap_remote_shell_command("zmx list"), "zmx list");
    }

    #[test]
    fn run_terminal_user_command_executes_as_silo() {
        assert_eq!(
            run_terminal_user_command("zmx list"),
            "sudo -iu silo bash -lc 'zmx list'"
        );
    }

    #[test]
    fn run_terminal_user_command_preserves_quoting() {
        assert_eq!(
            run_terminal_user_command("zmx history 'terminal-1' --vt"),
            "sudo -iu silo bash -lc 'zmx history '\"'\"'terminal-1'\"'\"' --vt'"
        );
    }

    #[test]
    fn workspace_shell_command_wraps_script_via_base64() {
        let command = workspace_shell_command("printf \"hi\\n\"");
        assert!(command.starts_with("sudo -iu silo bash -lc 'printf %s "));
        assert!(command.contains("| base64 --decode | bash"));
    }

    #[test]
    fn terminal_run_remote_command_passes_command_as_argument() {
        assert_eq!(
            terminal_run_remote_command("terminal-1", "codex -- \"hello\""),
            "sudo -iu silo bash -lc 'if [ -f '\"'\"'/home/silo/.silo/credentials.sh'\"'\"' ]; then source '\"'\"'/home/silo/.silo/credentials.sh'\"'\"'; fi; cd '\"'\"'/home/silo/workspace'\"'\"'; zmx run '\"'\"'terminal-1'\"'\"' '\"'\"'codex -- \"hello\"'\"'\"''"
        );
    }

    #[test]
    fn workspace_shell_command_with_credentials_sources_credentials_file() {
        let script = workspace_shell_script(
            Some(
                "if [ -f '/home/silo/.silo/credentials.sh' ]; then\n. '/home/silo/.silo/credentials.sh'\nfi",
            ),
            "git status --short",
        );

        assert!(script.contains("if [ -f '/home/silo/.silo/credentials.sh' ]; then"));
        assert!(script.contains(". '/home/silo/.silo/credentials.sh'"));
        assert!(script.contains("cd '/home/silo/workspace'"));
        assert!(script.contains("git status --short"));
    }

    #[test]
    fn bootstrap_git_command_inlines_auth_env() {
        assert_eq!(
            bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""),
            "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git -C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""
        );
    }

    #[test]
    fn bootstrap_retry_detects_broken_pipe() {
        assert!(should_retry_template_bootstrap(
            "failed to write gcloud ssh stdin: Broken pipe (os error 32)"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn stdin_write_error_preserves_remote_stderr_and_forces_failure() {
        let output = Output {
            status: ExitStatus::from_raw(0),
            stdout: Vec::new(),
            stderr: b"ssh: connect to host 1.2.3.4 port 22: Connection refused\n".to_vec(),
        };
        let error = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe");

        let result = command_result_with_stdin_write_error(output, &error);

        assert!(!result.success);
        assert!(result.stderr.contains("Connection refused"));
        assert!(result.stderr.contains("Broken pipe"));
    }

    #[test]
    fn persist_bootstrap_state_command_streams_base64_signature() {
        let command = persist_workspace_bootstrap_state_command("version=10\nworkspace=demo");

        assert!(command.contains("base64 --decode"));
        assert!(command.contains("/home/silo/.silo/workspace-bootstrap-state"));
        assert!(!command.contains("workspace=demo"));
    }

    #[test]
    fn truncate_scrollback_keeps_recent_tail() {
        let (scrollback, truncated) = truncate_scrollback("abcdef".to_string(), 3);
        assert!(truncated);
        assert_eq!(scrollback, "def");
    }

    #[test]
    fn attach_scrollback_mode_skips_when_requested() {
        assert_eq!(
            attach_scrollback_mode(Some(true)),
            AttachScrollbackMode::Skip
        );
    }

    #[test]
    fn attach_scrollback_mode_loads_by_default() {
        assert_eq!(attach_scrollback_mode(None), AttachScrollbackMode::Load);
        assert_eq!(
            attach_scrollback_mode(Some(false)),
            AttachScrollbackMode::Load
        );
    }

    #[test]
    fn terminal_command_bytes_appends_newline_once() {
        assert_eq!(terminal_command_bytes("pwd"), b"pwd\n");
        assert_eq!(terminal_command_bytes("pwd\n"), b"pwd\n");
        assert!(terminal_command_bytes("").is_empty());
    }

    #[test]
    fn normalize_workspace_relative_path_rejects_non_relative_paths() {
        assert_eq!(
            normalize_workspace_relative_path("apps/web/.env.local"),
            Some("apps/web/.env.local".to_string())
        );
        assert_eq!(
            normalize_workspace_relative_path(".env"),
            Some(".env".to_string())
        );
        assert_eq!(normalize_workspace_relative_path("../.env"), None);
        assert_eq!(normalize_workspace_relative_path("/tmp/.env"), None);
    }

    #[test]
    fn load_bootstrap_env_files_reads_and_hashes_configured_files() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join("apps/web")).expect("nested env dir should exist");
        fs::write(project_root.join(".env.local"), "ROOT=1\n").expect("root env should exist");
        fs::write(project_root.join("apps/web/.env"), "WEB=1\n").expect("nested env should exist");

        let project = ProjectConfig {
            name: "demo".to_string(),
            path: project_root.to_string_lossy().into_owned(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            env_files: vec![
                ".env.local".to_string(),
                "apps/web/.env".to_string(),
                "../ignored".to_string(),
            ],
            gcloud: Default::default(),
        };

        let env_files = load_bootstrap_env_files("demo", &project);

        assert_eq!(env_files.len(), 2);
        assert_eq!(env_files[0].relative_path, ".env.local");
        assert_eq!(env_files[0].contents_sha256, hex_sha256(b"ROOT=1\n"));
        assert_eq!(env_files[1].relative_path, "apps/web/.env");
        assert_eq!(env_files[1].contents_sha256, hex_sha256(b"WEB=1\n"));
    }

    #[test]
    fn workspace_env_file_sync_script_preserves_relative_paths() {
        let script = workspace_env_file_sync_script(&[BootstrapEnvFile {
            relative_path: "apps/web/.env.local".to_string(),
            contents_base64: BASE64_STANDARD.encode("WEB=1\n"),
            contents_sha256: "digest".to_string(),
        }]);

        assert!(script.contains("mkdir -p '/home/silo/workspace/apps/web'"));
        assert!(script.contains("base64 --decode > '/home/silo/workspace/apps/web/.env.local'"));
        assert!(script.contains("chmod 600 '/home/silo/workspace/apps/web/.env.local'"));
    }

    #[test]
    fn load_bootstrap_chrome_profile_hashes_only_syncable_profile_state() {
        let temp_dir = TempDir::new();
        let chrome_dir = temp_dir.path.join("Chrome");
        fs::create_dir_all(chrome_dir.join("Default/Extensions/uBlock"))
            .expect("chrome extensions dir should exist");
        fs::write(
            chrome_dir.join("Default/Preferences"),
            "{\"theme\":\"dark\"}",
        )
        .expect("preferences file should exist");
        fs::write(
            chrome_dir.join("Default/Extensions/uBlock/manifest.json"),
            "{\"name\":\"uBlock\"}",
        )
        .expect("extension payload should exist");

        let (source_dir, original_hash) =
            load_bootstrap_chrome_profile(&chrome_dir.to_string_lossy());
        assert_eq!(source_dir, chrome_dir.to_string_lossy());
        assert!(!original_hash.is_empty());

        fs::write(
            chrome_dir.join("Default/Extensions/uBlock/manifest.json"),
            "{\"name\":\"uBlock Origin\"}",
        )
        .expect("extension payload should update");
        let (_, hash_with_extension_change) =
            load_bootstrap_chrome_profile(&chrome_dir.to_string_lossy());
        assert_eq!(hash_with_extension_change, original_hash);

        fs::write(
            chrome_dir.join("Default/Preferences"),
            "{\"theme\":\"light\"}",
        )
        .expect("preferences should update");
        let (_, hash_with_real_change) =
            load_bootstrap_chrome_profile(&chrome_dir.to_string_lossy());
        assert_ne!(hash_with_real_change, original_hash);
    }

    #[test]
    fn chrome_profile_sync_paths_include_profile_state_but_not_extensions_or_caches() {
        let sync_paths = CHROME_PROFILE_SYNC_PATHS
            .iter()
            .copied()
            .collect::<HashSet<_>>();

        assert!(sync_paths.contains("Local State"));
        assert!(sync_paths.contains("Default/Preferences"));
        assert!(sync_paths.contains("Default/Login Data"));
        assert!(sync_paths.contains("Default/Cookies"));
        assert!(!sync_paths.contains("Default/Extensions"));
        assert!(!sync_paths.contains("component_crx_cache"));
        assert!(!sync_paths.contains("extensions_crx_cache"));
        assert!(!sync_paths.contains("screen_ai"));
    }

    #[test]
    fn chrome_profile_remote_sync_command_targets_google_chrome_dir() {
        let command = chrome_profile_remote_sync_command();
        assert!(command.contains(REMOTE_CHROME_USER_DATA_DIR));
        assert!(command.contains("tar -xzf -"));
        assert!(command.contains("rm -rf"));
    }

    #[test]
    fn workspace_bootstrap_signature_includes_chrome_profile_state() {
        let bootstrap = WorkspaceBootstrap {
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            workspace_branch: Some("feature/test".to_string()),
            gh_username: "octocat".to_string(),
            gh_token: "gh-token".to_string(),
            chrome_user_data_dir: "/Users/test/Library/Application Support/Google/Chrome"
                .to_string(),
            chrome_user_data_hash: "chrome-digest".to_string(),
            codex_token: "codex-token".to_string(),
            claude_token: "claude-token".to_string(),
            git_user_name: "Test User".to_string(),
            git_user_email: "test@example.com".to_string(),
            env_files: Vec::new(),
        };

        let signature = workspace_bootstrap_signature("demo-silo-workspace", &bootstrap);

        assert!(signature.contains(
            "chrome_user_data_dir=/Users/test/Library/Application Support/Google/Chrome"
        ));
        assert!(signature.contains("chrome_user_data_hash=chrome-digest"));
    }

    #[test]
    fn codex_auth_json_contains_access_token() {
        let payload = codex_auth_json("codex-token");
        assert!(payload.contains("\"access_token\":\"codex-token\""));
        assert!(payload.contains("\"auth_mode\":\"chatgpt\""));
    }

    #[test]
    fn claude_state_json_marks_workspace_as_trusted_and_onboarded() {
        let payload = claude_state_json();
        assert!(payload.contains(TERMINAL_WORKSPACE_DIR));
        assert!(payload.contains("\"hasTrustDialogAccepted\":true"));
        assert!(payload.contains("\"hasCompletedOnboarding\":true"));
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
                working: None,
                unread: None,
            },
            WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
                attachment_id: "terminal-1741812345678".to_string(),
                working: None,
                unread: None,
            },
            WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
                attachment_id: "terminal-1741812345679".to_string(),
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
    fn mutate_sessions_mark_read_recomputes_workspace_unread() {
        let metadata = mutate_sessions(
            &[WorkspaceSession {
                kind: "terminal".to_string(),
                name: "codex".to_string(),
                attachment_id: "terminal-1".to_string(),
                working: Some(false),
                unread: Some(true),
            }],
            Some("2026-03-14T00:00:00Z"),
            SessionMutation::MarkRead("terminal-1"),
        );

        assert!(!metadata.unread);
        assert_eq!(metadata.working, None);
        assert_eq!(
            metadata.last_working.as_deref(),
            Some("2026-03-14T00:00:00Z")
        );
        assert_eq!(metadata.sessions[0].unread, Some(false));
    }

    #[test]
    fn mutate_sessions_remove_drops_session_and_working_flag() {
        let metadata = mutate_sessions(
            &[WorkspaceSession {
                kind: "terminal".to_string(),
                name: "codex".to_string(),
                attachment_id: "terminal-1".to_string(),
                working: Some(true),
                unread: Some(false),
            }],
            Some("2026-03-14T00:00:00Z"),
            SessionMutation::Remove("terminal-1"),
        );

        assert!(metadata.sessions.is_empty());
        assert_eq!(metadata.working, None);
        assert_eq!(
            metadata.last_working.as_deref(),
            Some("2026-03-14T00:00:00Z")
        );
    }

    #[test]
    fn mutate_sessions_updates_last_working_when_session_is_working() {
        let metadata = mutate_sessions(
            &[WorkspaceSession {
                kind: "terminal".to_string(),
                name: "codex".to_string(),
                attachment_id: "terminal-1".to_string(),
                working: Some(false),
                unread: Some(false),
            }],
            Some("2026-03-14T00:00:00Z"),
            SessionMutation::Upsert(WorkspaceSession {
                kind: "terminal".to_string(),
                name: "codex".to_string(),
                attachment_id: "terminal-1".to_string(),
                working: Some(true),
                unread: Some(false),
            }),
        );

        assert_eq!(metadata.working, Some(true));
        assert!(metadata.last_working.is_some());
        assert_ne!(
            metadata.last_working.as_deref(),
            Some("2026-03-14T00:00:00Z")
        );
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
            output: Mutex::new(Channel::new(|_| Ok(()))),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
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
            output: Mutex::new(Channel::new(|_| Ok(()))),
            window_label: Mutex::new("main".to_string()),
            connected: Mutex::new(false),
            connected_cv: Condvar::new(),
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

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = format!(
                "silo-terminal-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(Path::new(&self.path));
        }
    }
}
