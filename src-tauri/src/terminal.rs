use crate::workspaces::{self, WorkspaceLookup};
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, EventTarget, State, WebviewWindow};
use uuid::Uuid;

const TERMINAL_EXIT_EVENT: &str = "terminal://exit";
const TERMINAL_ERROR_EVENT: &str = "terminal://error";
const DEFAULT_TERMINAL_COLS: u16 = 80;
const DEFAULT_TERMINAL_ROWS: u16 = 24;
const MAX_SCROLLBACK_BYTES: usize = 512 * 1024;
const ATTACH_COMMAND_WAIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalSessionSummary {
    name: String,
    pid: Option<u32>,
    clients: u32,
    started_in: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalAttachResult {
    attachment_id: String,
    session: TerminalSessionSummary,
    scrollback_vt: String,
    scrollback_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TerminalRunResult {
    session: TerminalSessionSummary,
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
struct TerminalExitPayload {
    attachment_id: String,
    exit_code: u32,
    signal: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct TerminalErrorPayload {
    attachment_id: String,
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
}

#[derive(Clone, Default)]
pub struct TerminalManager {
    inner: Arc<Mutex<AttachmentRegistry>>,
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
}

#[derive(Debug)]
struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

#[tauri::command]
pub async fn terminal_list_terminals(
    workspace: String,
) -> Result<Vec<TerminalSessionSummary>, String> {
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
    name: String,
    command: Option<String>,
    output: Channel<Vec<u8>>,
) -> Result<TerminalAttachResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    let key = AttachmentKey {
        workspace: workspace.clone(),
        name: name.clone(),
    };
    let (scrollback_vt, scrollback_truncated) = load_scrollback(&lookup, &name).await?;

    if let Some(existing) = state.get_by_key(&key) {
        if let Ok(mut current_output) = existing.output.lock() {
            *current_output = output;
        }
        if let Ok(mut current_window) = existing.window_label.lock() {
            *current_window = window.label().to_string();
        }
        if let Some(command) = command {
            queue_attach_command(existing.clone(), command);
        }

        return Ok(TerminalAttachResult {
            attachment_id: existing.id.clone(),
            session: resolve_attached_session(&lookup, &name).await?,
            scrollback_vt,
            scrollback_truncated,
        });
    }

    let attachment = spawn_terminal_attachment(
        app,
        state.inner().clone(),
        lookup.clone(),
        key,
        output,
        window.label().to_string(),
    )?;
    if let Some(command) = command {
        queue_attach_command(attachment.clone(), command);
    }

    Ok(TerminalAttachResult {
        attachment_id: attachment.id.clone(),
        session: resolve_attached_session(&lookup, &name).await?,
        scrollback_vt,
        scrollback_truncated,
    })
}

#[tauri::command]
pub async fn terminal_run_terminal(
    workspace: String,
    name: String,
    command: String,
) -> Result<TerminalRunResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    let created = find_terminal_session(&lookup, &name).await?.is_none();
    let mut payload = command.into_bytes();
    if !payload.ends_with(b"\n") {
        payload.push(b'\n');
    }

    let result =
        run_remote_command_with_stdin(&lookup, &format!("zmx run {}", shell_quote(&name)), payload)
            .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to run terminal command",
            &result.stderr,
        ));
    }

    let session = find_terminal_session(&lookup, &name)
        .await?
        .ok_or_else(|| format!("terminal session not found after run: {name}"))?;

    Ok(TerminalRunResult { session, created })
}

#[tauri::command]
pub fn terminal_detach_terminal(
    state: State<'_, TerminalManager>,
    workspace: String,
    name: String,
) -> Result<TerminalDetachResult, String> {
    let key = AttachmentKey { workspace, name };
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
    name: String,
) -> Result<TerminalKillResult, String> {
    let lookup = workspaces::find_workspace(&workspace).await?;
    let result = run_remote_command(&lookup, &format!("zmx kill {}", shell_quote(&name))).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to kill terminal session",
            &result.stderr,
        ));
    }

    let key = AttachmentKey { workspace, name };
    if let Some(attachment) = state.remove_by_key(&key) {
        kill_local_attachment(&attachment)?;
    }

    Ok(TerminalKillResult { killed: true })
}

#[tauri::command]
pub fn terminal_write_terminal(
    state: State<'_, TerminalManager>,
    attachment_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let attachment = state
        .get_by_id(&attachment_id)
        .ok_or_else(|| format!("terminal attachment not found: {attachment_id}"))?;
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
    attachment_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let attachment = state
        .get_by_id(&attachment_id)
        .ok_or_else(|| format!("terminal attachment not found: {attachment_id}"))?;
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
            wrap_remote_shell_command(&format!("exec zmx attach {}", shell_quote(&key.name)))
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
                    attachment_id: attachment.id.clone(),
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
                    attachment_id: attachment.id.clone(),
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
                attachment_id: attachment.id.clone(),
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
) -> Result<Vec<TerminalSessionSummary>, String> {
    let result = run_remote_command(lookup, "zmx list").await?;
    if !result.success {
        let stderr = result.stderr.trim();
        if stderr.contains("not found") || stderr.contains("command not found") {
            return Ok(Vec::new());
        }

        return Err(remote_command_error(
            "failed to list terminal sessions",
            &result.stderr,
        ));
    }

    parse_terminal_sessions(&result.stdout)
}

async fn find_terminal_session(
    lookup: &WorkspaceLookup,
    name: &str,
) -> Result<Option<TerminalSessionSummary>, String> {
    Ok(list_terminals_in_workspace(lookup)
        .await?
        .into_iter()
        .find(|session| session.name == name))
}

async fn load_scrollback(lookup: &WorkspaceLookup, name: &str) -> Result<(String, bool), String> {
    let result =
        run_remote_command(lookup, &format!("zmx history {} --vt", shell_quote(name))).await?;
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

async fn resolve_attached_session(
    lookup: &WorkspaceLookup,
    name: &str,
) -> Result<TerminalSessionSummary, String> {
    for _ in 0..5 {
        if let Some(session) = find_terminal_session(lookup, name).await? {
            return Ok(session);
        }

        std::thread::sleep(Duration::from_millis(150));
    }

    Ok(pending_terminal_session(name))
}

async fn run_remote_command(
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

        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin_bytes.is_some() {
            command.stdin(Stdio::piped());
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to execute gcloud ssh: {error}"))?;

        if let Some(stdin_bytes) = stdin_bytes {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(&stdin_bytes)
                    .map_err(|error| format!("failed to write gcloud ssh stdin: {error}"))?;
            }
        }

        let output = child
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

fn wrap_remote_shell_command(command: &str) -> String {
    format!("bash -lc {}", shell_quote(command))
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

fn pending_terminal_session(name: &str) -> TerminalSessionSummary {
    TerminalSessionSummary {
        name: name.to_string(),
        pid: None,
        clients: 0,
        started_in: None,
        created_at: None,
    }
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

fn parse_terminal_sessions(stdout: &str) -> Result<Vec<TerminalSessionSummary>, String> {
    let mut sessions = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        sessions.push(parse_terminal_session(line)?);
    }

    Ok(sessions)
}

fn parse_terminal_session(line: &str) -> Result<TerminalSessionSummary, String> {
    let mut name = None;
    let mut pid = None;
    let mut clients = 0u32;
    let mut started_in = None;
    let mut created_at = None;

    for field in line.split('\t') {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };

        match key {
            "session_name" => name = Some(value.to_string()),
            "pid" => {
                pid = value.parse::<u32>().ok();
            }
            "clients" => {
                clients = value.parse::<u32>().unwrap_or(0);
            }
            "started_in" => started_in = non_empty(value),
            "created" | "created_at" => created_at = non_empty(value),
            _ => {}
        }
    }

    let name = name.ok_or_else(|| format!("invalid zmx list row missing session name: {line}"))?;
    Ok(TerminalSessionSummary {
        name,
        pid,
        clients,
        started_in,
        created_at,
    })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
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

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("ab'cd"), "'ab'\"'\"'cd'");
    }

    #[test]
    fn wrap_remote_shell_command_quotes_inner_command() {
        assert_eq!(wrap_remote_shell_command("zmx list"), "bash -lc 'zmx list'");
    }

    #[test]
    fn truncate_scrollback_keeps_recent_tail() {
        let (scrollback, truncated) = truncate_scrollback("abcdef".to_string(), 3);
        assert!(truncated);
        assert_eq!(scrollback, "def");
    }

    #[test]
    fn terminal_command_bytes_appends_newline_once() {
        assert_eq!(terminal_command_bytes("pwd"), b"pwd\n");
        assert_eq!(terminal_command_bytes("pwd\n"), b"pwd\n");
        assert!(terminal_command_bytes("").is_empty());
    }

    #[test]
    fn parse_terminal_session_reads_known_fields() {
        let session = parse_terminal_session(
            "session_name=dev\tpid=42\tclients=2\tcreated=2025-01-01T00:00:00Z\tstarted_in=/tmp",
        )
        .expect("session should parse");
        assert_eq!(
            session,
            TerminalSessionSummary {
                name: "dev".to_string(),
                pid: Some(42),
                clients: 2,
                started_in: Some("/tmp".to_string()),
                created_at: Some("2025-01-01T00:00:00Z".to_string()),
            }
        );
    }

    #[test]
    fn parse_terminal_sessions_ignores_empty_lines() {
        let sessions =
            parse_terminal_sessions("\nsession_name=dev\tpid=42\tclients=0\tstarted_in=/tmp\n\n")
                .expect("sessions should parse");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "dev");
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
