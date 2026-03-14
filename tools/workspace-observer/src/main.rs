use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const COMPLETION_DEBOUNCE: Duration = Duration::from_millis(1500);
const FIFO_MODE: u32 = 0o622;
const METADATA_KEY: &str = "silo_state";

fn main() {
    if let Err(error) = run() {
        eprintln!("workspace-observer: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first().map(String::as_str) else {
        return Err("missing command".to_string());
    };

    match command {
        "daemon" => run_daemon(&args[1..]),
        "emit" => run_emit(&args[1..]),
        "mark-read" => run_mark_read(&args[1..]),
        "assistant-proxy" => run_assistant_proxy(&args[1..]),
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_daemon(args: &[String]) -> Result<(), String> {
    let options = DaemonOptions::parse(args)?;
    let runtime = RuntimePaths::new();
    runtime.ensure()?;
    if !acquire_pidfile(&runtime.pidfile)? {
        return Ok(());
    }

    ensure_fifo(&runtime.fifo)?;
    let (event_tx, event_rx) = std::sync::mpsc::channel::<ObserverEvent>();
    spawn_fifo_reader(runtime.fifo.clone(), event_tx.clone());

    let mut state = load_state(&runtime.state_file).unwrap_or_default();
    let mut last_published = None::<String>;
    let metadata = ComputeMetadataClient::new(
        options.project.clone(),
        options.zone.clone(),
        options.instance.clone(),
    );

    loop {
        while let Ok(event) = event_rx.try_recv() {
            apply_event(&mut state, event);
        }

        let live_sessions = list_zmx_sessions();
        reconcile_sessions(&mut state, &live_sessions);
        state.branch = read_workspace_branch();
        persist_state(&runtime.state_file, &state)?;

        let published = build_published_state(&state, &live_sessions);
        let published_json =
            serde_json::to_string(&published).map_err(|error| error.to_string())?;

        if last_published.as_deref() != Some(published_json.as_str()) {
            if let Err(error) = metadata.publish(&published_json, published.branch.as_deref()) {
                eprintln!("workspace-observer: failed to publish metadata: {error}");
            } else {
                last_published = Some(published_json);
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn run_emit(args: &[String]) -> Result<(), String> {
    let event = parse_emit_event(args)?;
    send_event(&RuntimePaths::new().fifo, &event)
}

fn run_mark_read(args: &[String]) -> Result<(), String> {
    let session = required_flag_value(args, "--session")?;
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::MarkRead {
            session: session.to_string(),
        },
    )
}

fn run_assistant_proxy(args: &[String]) -> Result<(), String> {
    let provider = required_flag_value(args, "--provider")?;
    let provider = AssistantProvider::parse(provider)
        .ok_or_else(|| format!("unsupported assistant provider: {provider}"))?;
    let command_start = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or_else(|| "assistant-proxy requires `--` before the wrapped command".to_string())?;
    let command = args[command_start + 1..].to_vec();
    if command.is_empty() {
        return Err("assistant-proxy requires a wrapped command".to_string());
    }

    let session = env::var("ZMX_SESSION").unwrap_or_default();
    if session.trim().is_empty() {
        return spawn_passthrough(command);
    }

    let runtime = RuntimePaths::new();
    let pty_system = native_pty_system();
    let (cols, rows) = current_terminal_size();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("failed to open pty: {error}"))?;

    let mut builder = CommandBuilder::new(&command[0]);
    if command.len() > 1 {
        builder.args(&command[1..]);
    }
    builder.env("ZMX_SESSION", &session);
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| format!("failed to spawn assistant command: {error}"))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("failed to open pty reader: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("failed to open pty writer: {error}"))?;

    let tracker = AssistantTracker::new(session.clone(), provider, runtime.fifo);
    let reader_tracker = Arc::clone(&tracker);
    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_done_signal = Arc::clone(&reader_done);
    let reader_thread = thread::spawn(move || {
        proxy_output(reader, io::stdout(), reader_tracker);
        reader_done_signal.store(true, Ordering::Relaxed);
    });

    let raw_mode = RawModeGuard::new().map_err(|error| error.to_string())?;
    let input_result = proxy_input(io::stdin(), writer, Arc::clone(&tracker));
    drop(raw_mode);

    tracker.finish_turn_if_needed();
    if input_result.is_err() {
        let _ = child.kill();
    }

    while !reader_done.load(Ordering::Relaxed) {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let _ = reader_thread.join();
    tracker.stop();
    let status = child
        .wait()
        .map_err(|error| format!("assistant command wait failed: {error}"))?;

    let code = status.exit_code();
    if code == 0 {
        return Ok(());
    }

    process::exit(code as i32);
}

fn spawn_passthrough(command: Vec<String>) -> Result<(), String> {
    let status = Command::new(&command[0])
        .args(&command[1..])
        .status()
        .map_err(|error| format!("failed to run wrapped command: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        process::exit(status.code().unwrap_or(1));
    }
}

fn proxy_input<R: Read>(
    mut stdin: R,
    mut writer: Box<dyn Write + Send>,
    tracker: Arc<AssistantTracker>,
) -> Result<(), String> {
    let mut buffer = [0u8; 4096];
    loop {
        let count = stdin
            .read(&mut buffer)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        if count == 0 {
            return Ok(());
        }
        let chunk = &buffer[..count];
        writer
            .write_all(chunk)
            .map_err(|error| format!("failed to write pty input: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("failed to flush pty input: {error}"))?;
        tracker.record_input(&String::from_utf8_lossy(chunk));
    }
}

fn proxy_output<R: Read, W: Write>(mut reader: R, mut stdout: W, tracker: Arc<AssistantTracker>) {
    let mut buffer = [0u8; 8192];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        let chunk = &buffer[..count];
        if stdout.write_all(chunk).is_err() {
            break;
        }
        if stdout.flush().is_err() {
            break;
        }
        tracker.record_output(count);
    }
}

#[derive(Debug, Clone)]
struct RuntimePaths {
    root: PathBuf,
    fifo: PathBuf,
    pidfile: PathBuf,
    state_file: PathBuf,
}

impl RuntimePaths {
    fn new() -> Self {
        let root = PathBuf::from("/home/silo/.silo/workspace-observer");
        Self {
            fifo: root.join("events.fifo"),
            pidfile: root.join("daemon.pid"),
            state_file: root.join("state.json"),
            root,
        }
    }

    fn ensure(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "failed to create observer runtime dir {}: {error}",
                self.root.display()
            )
        })?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to set runtime dir permissions: {error}"))
    }
}

#[derive(Debug, Clone)]
struct DaemonOptions {
    instance: String,
    project: String,
    zone: String,
}

impl DaemonOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        Ok(Self {
            instance: required_flag_value(args, "--instance")?.to_string(),
            project: required_flag_value(args, "--project")?.to_string(),
            zone: required_flag_value(args, "--zone")?.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ObserverState {
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionState {
    #[serde(default)]
    active_command: Option<String>,
    #[serde(default)]
    assistant_provider: Option<AssistantProvider>,
    #[serde(default)]
    working: bool,
    #[serde(default)]
    unread: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PublishedState {
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    working: bool,
    unread: bool,
    processes: Vec<PublishedProcess>,
}

#[derive(Debug, Clone, Serialize)]
struct PublishedProcess {
    #[serde(rename = "type")]
    kind: &'static str,
    name: String,
    attachment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    working: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unread: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum AssistantProvider {
    Codex,
    Claude,
}

impl AssistantProvider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ObserverEvent {
    ShellCommandStarted {
        session: String,
        command: String,
    },
    ShellCommandFinished {
        session: String,
    },
    AssistantPromptSubmitted {
        session: String,
        provider: AssistantProvider,
    },
    AssistantTurnCompleted {
        session: String,
        provider: AssistantProvider,
    },
    MarkRead {
        session: String,
    },
}

#[derive(Debug, Clone, Default)]
struct ZmxSession {
    name: String,
    command: Option<String>,
}

fn apply_event(state: &mut ObserverState, event: ObserverEvent) {
    match event {
        ObserverEvent::ShellCommandStarted { session, command } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(sanitize_command_name(&command));
            session_state.assistant_provider = resolve_assistant_provider(&command);
            session_state.working = false;
        }
        ObserverEvent::ShellCommandFinished { session } => {
            if let Some(session_state) = state.sessions.get_mut(&session) {
                session_state.active_command = None;
                session_state.assistant_provider = None;
                session_state.working = false;
            }
        }
        ObserverEvent::AssistantPromptSubmitted { session, provider } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(provider.command_name().to_string());
            session_state.assistant_provider = Some(provider);
            session_state.working = true;
            session_state.unread = false;
        }
        ObserverEvent::AssistantTurnCompleted { session, provider } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(provider.command_name().to_string());
            session_state.assistant_provider = Some(provider);
            session_state.working = false;
            session_state.unread = true;
        }
        ObserverEvent::MarkRead { session } => {
            if let Some(session_state) = state.sessions.get_mut(&session) {
                session_state.unread = false;
            }
        }
    }
}

fn reconcile_sessions(state: &mut ObserverState, live_sessions: &[ZmxSession]) {
    let live = live_sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect::<Vec<_>>();

    state
        .sessions
        .retain(|name, _| live.iter().any(|candidate| *candidate == name));

    for live_session in live_sessions {
        let session_state = state.sessions.entry(live_session.name.clone()).or_default();
        if session_state.active_command.is_none() {
            session_state.active_command =
                live_session.command.as_deref().map(sanitize_command_name);
            if let Some(command) = &session_state.active_command {
                session_state.assistant_provider = resolve_assistant_provider(command);
            }
        }
    }
}

fn build_published_state(state: &ObserverState, live_sessions: &[ZmxSession]) -> PublishedState {
    let live_map = live_sessions
        .iter()
        .map(|session| (session.name.clone(), session))
        .collect::<HashMap<_, _>>();
    let mut working = false;
    let mut unread = false;
    let mut processes = state
        .sessions
        .iter()
        .filter_map(|(session_name, session_state)| {
            let live_session = live_map.get(session_name)?;
            let name = session_state
                .active_command
                .clone()
                .or_else(|| live_session.command.clone())
                .unwrap_or_else(|| "shell".to_string());
            let assistant_capable = session_state.assistant_provider.is_some()
                || resolve_assistant_provider(&name).is_some();
            working |= session_state.working;
            unread |= session_state.unread;

            Some(PublishedProcess {
                kind: "terminal",
                name,
                attachment_id: session_name.clone(),
                working: assistant_capable.then_some(session_state.working),
                unread: assistant_capable.then_some(session_state.unread),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));

    PublishedState {
        branch: state.branch.clone(),
        working,
        unread,
        processes,
    }
}

fn load_state(path: &Path) -> Result<ObserverState, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read state file {}: {error}", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| format!("invalid state json: {error}"))
}

fn persist_state(path: &Path, state: &ObserverState) -> Result<(), String> {
    let temp_path = path.with_extension("tmp");
    let contents = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&temp_path, contents).map_err(|error| {
        format!(
            "failed to write state file {}: {error}",
            temp_path.display()
        )
    })?;
    fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to set state permissions: {error}"))?;
    fs::rename(&temp_path, path).map_err(|error| {
        format!(
            "failed to move state file {} into place: {error}",
            path.display()
        )
    })
}

fn ensure_fifo(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }

    let path_cstring = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|error| format!("invalid fifo path: {error}"))?;
    let result = unsafe { libc::mkfifo(path_cstring.as_ptr(), FIFO_MODE as libc::mode_t) };
    if result == -1 {
        return Err(format!(
            "failed to create event fifo {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }

    fs::set_permissions(path, fs::Permissions::from_mode(FIFO_MODE))
        .map_err(|error| format!("failed to set fifo permissions: {error}"))
}

fn spawn_fifo_reader(path: PathBuf, tx: std::sync::mpsc::Sender<ObserverEvent>) {
    thread::spawn(move || loop {
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(_) => {
                thread::sleep(Duration::from_millis(250));
                continue;
            }
        };

        let mut reader = io::BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match io::BufRead::read_line(&mut reader, &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let payload = line.trim();
                    if payload.is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<ObserverEvent>(payload) {
                        let _ = tx.send(event);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn send_event(path: &Path, event: &ObserverEvent) -> Result<(), String> {
    let payload = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let path_cstring = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|error| format!("invalid fifo path: {error}"))?;
    let fd = unsafe { libc::open(path_cstring.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
    if fd == -1 {
        return Err(format!(
            "failed to open event fifo {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }

    let bytes = format!("{payload}\n").into_bytes();
    let result = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    let close_result = unsafe { libc::close(fd) };
    if result == -1 {
        return Err(format!(
            "failed to write event fifo {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }
    if close_result == -1 {
        return Err(format!(
            "failed to close event fifo {}: {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }

    Ok(())
}

fn parse_emit_event(args: &[String]) -> Result<ObserverEvent, String> {
    let kind = required_flag_value(args, "--kind")?;
    let session = required_flag_value(args, "--session")?.to_string();

    match kind {
        "shell_command_started" => Ok(ObserverEvent::ShellCommandStarted {
            session,
            command: required_flag_value(args, "--command")?.to_string(),
        }),
        "shell_command_finished" => Ok(ObserverEvent::ShellCommandFinished { session }),
        "assistant_prompt_submitted" => Ok(ObserverEvent::AssistantPromptSubmitted {
            session,
            provider: AssistantProvider::parse(required_flag_value(args, "--provider")?)
                .ok_or_else(|| "invalid assistant provider".to_string())?,
        }),
        "assistant_turn_completed" => Ok(ObserverEvent::AssistantTurnCompleted {
            session,
            provider: AssistantProvider::parse(required_flag_value(args, "--provider")?)
                .ok_or_else(|| "invalid assistant provider".to_string())?,
        }),
        "mark_read" => Ok(ObserverEvent::MarkRead { session }),
        other => Err(format!("unsupported event kind: {other}")),
    }
}

fn required_flag_value<'a>(args: &'a [String], flag: &str) -> Result<&'a str, String> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("missing required flag: {flag}"))?;
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("missing value for flag: {flag}"))
}

fn sanitize_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "shell".to_string();
    }

    trimmed.chars().take(200).collect()
}

fn resolve_assistant_provider(command: &str) -> Option<AssistantProvider> {
    let token = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match token.as_str() {
        "codex" => Some(AssistantProvider::Codex),
        "claude" | "cc" => Some(AssistantProvider::Claude),
        _ => None,
    }
}

fn list_zmx_sessions() -> Vec<ZmxSession> {
    let output = match Command::new("zmx").arg("list").output() {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_zmx_session)
        .collect()
}

fn parse_zmx_session(line: &str) -> Option<ZmxSession> {
    let mut name = None;
    let mut command = None;
    for field in line.split('\t') {
        let (key, value) = field.split_once('=')?;
        match key {
            "session_name" => name = Some(value.to_string()),
            "cmd" => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    command = Some(trimmed.to_string());
                }
            }
            _ => {}
        }
    }

    Some(ZmxSession {
        name: name?,
        command,
    })
}

fn read_workspace_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["-C", "/home/silo/workspace", "branch", "--show-current"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}

fn acquire_pidfile(path: &Path) -> Result<bool, String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if let Ok(pid) = existing.trim().parse::<i32>() {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if alive {
                return Ok(false);
            }
        }
    }

    fs::write(path, process::id().to_string())
        .map_err(|error| format!("failed to write pidfile {}: {error}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("failed to set pidfile permissions: {error}"))?;
    Ok(true)
}

struct ComputeMetadataClient {
    project: String,
    zone: String,
    instance: String,
    client: Client,
}

impl ComputeMetadataClient {
    fn new(project: String, zone: String, instance: String) -> Self {
        Self {
            project,
            zone,
            instance,
            client: Client::builder()
                .build()
                .expect("reqwest blocking client should build"),
        }
    }

    fn publish(&self, state_json: &str, branch: Option<&str>) -> Result<(), String> {
        let token = self.fetch_access_token()?;
        let (fingerprint, mut items) = self.fetch_instance_metadata(&token)?;
        items.insert(METADATA_KEY.to_string(), state_json.to_string());
        if let Some(branch) = branch {
            items.insert("branch".to_string(), branch.to_string());
        }

        let items = items
            .into_iter()
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect::<Vec<_>>();
        let body = json!({
            "fingerprint": fingerprint,
            "items": items,
        });

        let url = format!(
            "https://compute.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}/setMetadata",
            self.project, self.zone, self.instance
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .map_err(|error| format!("failed to call setMetadata: {error}"))?;
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().unwrap_or_default();
        Err(format!("setMetadata failed with status {status}: {body}"))
    }

    fn fetch_access_token(&self) -> Result<String, String> {
        let response = self
            .client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .header("Metadata-Flavor", "Google")
            .send()
            .map_err(|error| format!("failed to get metadata access token: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(format!(
                "metadata access token request failed with status {status}: {body}"
            ));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        response
            .json::<TokenResponse>()
            .map(|response| response.access_token)
            .map_err(|error| format!("failed to parse metadata access token response: {error}"))
    }

    fn fetch_instance_metadata(
        &self,
        token: &str,
    ) -> Result<(String, BTreeMap<String, String>), String> {
        let url = format!(
            "https://compute.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}",
            self.project, self.zone, self.instance
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|error| format!("failed to fetch instance metadata: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(format!(
                "instance metadata fetch failed with status {status}: {body}"
            ));
        }

        let value = response
            .json::<Value>()
            .map_err(|error| format!("failed to parse instance metadata response: {error}"))?;
        let fingerprint = value
            .get("metadata")
            .and_then(|metadata| metadata.get("fingerprint"))
            .and_then(Value::as_str)
            .ok_or_else(|| "instance metadata response is missing fingerprint".to_string())?
            .to_string();
        let items = value
            .get("metadata")
            .and_then(|metadata| metadata.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut map = BTreeMap::new();
        for item in items {
            let Some(key) = item.get("key").and_then(Value::as_str) else {
                continue;
            };
            let Some(value) = item.get("value").and_then(Value::as_str) else {
                continue;
            };
            map.insert(key.to_string(), value.to_string());
        }

        Ok((fingerprint, map))
    }
}

struct AssistantTracker {
    provider: AssistantProvider,
    session: String,
    fifo: PathBuf,
    state: Mutex<AssistantTrackerState>,
    wake: Condvar,
    stopped: AtomicBool,
}

#[derive(Default)]
struct AssistantTrackerState {
    awaiting_turn: bool,
    input_buffer: String,
    deadline: Option<Instant>,
}

impl AssistantTracker {
    fn new(session: String, provider: AssistantProvider, fifo: PathBuf) -> Arc<Self> {
        let tracker = Arc::new(Self {
            provider,
            session,
            fifo,
            state: Mutex::new(AssistantTrackerState::default()),
            wake: Condvar::new(),
            stopped: AtomicBool::new(false),
        });

        let completion_tracker = Arc::clone(&tracker);
        thread::spawn(move || completion_tracker.completion_loop());
        tracker
    }

    fn record_input(&self, input: &str) {
        let prompts = {
            let mut state = self.state.lock().expect("tracker lock should not poison");
            let prompts = collect_submitted_assistant_prompts(&mut state.input_buffer, input);
            if !prompts.is_empty() {
                state.awaiting_turn = true;
                state.deadline = None;
            }
            prompts
        };

        if prompts.is_empty() {
            return;
        }

        let _ = send_event(
            &self.fifo,
            &ObserverEvent::AssistantPromptSubmitted {
                session: self.session.clone(),
                provider: self.provider,
            },
        );
        self.wake.notify_all();
    }

    fn record_output(&self, count: usize) {
        if count == 0 {
            return;
        }

        let mut state = self.state.lock().expect("tracker lock should not poison");
        if !state.awaiting_turn {
            return;
        }
        state.deadline = Some(Instant::now() + COMPLETION_DEBOUNCE);
        self.wake.notify_all();
    }

    fn finish_turn_if_needed(&self) {
        let should_emit = {
            let mut state = self.state.lock().expect("tracker lock should not poison");
            if !state.awaiting_turn {
                false
            } else {
                state.awaiting_turn = false;
                state.input_buffer.clear();
                state.deadline = None;
                true
            }
        };

        if should_emit {
            let _ = send_event(
                &self.fifo,
                &ObserverEvent::AssistantTurnCompleted {
                    session: self.session.clone(),
                    provider: self.provider,
                },
            );
        }
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.wake.notify_all();
    }

    fn completion_loop(self: Arc<Self>) {
        loop {
            let mut state = self.state.lock().expect("tracker lock should not poison");
            while !self.stopped.load(Ordering::Relaxed) && state.deadline.is_none() {
                state = self.wake.wait(state).expect("condvar wait should succeed");
            }
            if self.stopped.load(Ordering::Relaxed) {
                return;
            }

            let Some(deadline) = state.deadline else {
                continue;
            };
            let now = Instant::now();
            if now < deadline {
                let timeout = deadline - now;
                let (next_state, _) = self
                    .wake
                    .wait_timeout(state, timeout)
                    .expect("condvar timeout wait should succeed");
                drop(next_state);
                continue;
            }

            if !state.awaiting_turn {
                state.deadline = None;
                continue;
            }

            state.awaiting_turn = false;
            state.deadline = None;
            state.input_buffer.clear();
            drop(state);

            let _ = send_event(
                &self.fifo,
                &ObserverEvent::AssistantTurnCompleted {
                    session: self.session.clone(),
                    provider: self.provider,
                },
            );
        }
    }
}

struct RawModeGuard {
    original: Option<libc::termios>,
}

impl RawModeGuard {
    fn new() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        if unsafe { libc::isatty(fd) } != 1 {
            return Ok(Self { original: None });
        }

        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } == -1 {
            return Err(io::Error::last_os_error());
        }

        let original = termios;
        unsafe { libc::cfmakeraw(&mut termios) };
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            original: Some(original),
        })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let Some(original) = self.original else {
            return;
        };
        let fd = io::stdin().as_raw_fd();
        let _ = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
    }
}

fn current_terminal_size() -> (u16, u16) {
    let fd = io::stdout().as_raw_fd();
    let mut winsize = unsafe { std::mem::zeroed::<libc::winsize>() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsize) } == -1 {
        return (DEFAULT_COLS, DEFAULT_ROWS);
    }

    let cols = if winsize.ws_col == 0 {
        DEFAULT_COLS
    } else {
        winsize.ws_col
    };
    let rows = if winsize.ws_row == 0 {
        DEFAULT_ROWS
    } else {
        winsize.ws_row
    };
    (cols, rows)
}

fn normalize_assistant_input(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        let current = chars[index];
        if current != '\u{001b}' {
            output.push(current);
            index += 1;
            continue;
        }

        let next = chars.get(index + 1).copied();
        if next == Some('[') {
            index += 2;
            while index < chars.len() {
                let control = chars[index];
                if ('@'..='~').contains(&control) {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }

        if next == Some(']') {
            index += 2;
            while index < chars.len() {
                let control = chars[index];
                if control == '\u{0007}' {
                    index += 1;
                    break;
                }
                if control == '\u{001b}' && chars.get(index + 1) == Some(&'\\') {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }

        index += if next.is_some() { 2 } else { 1 };
    }

    output
}

fn collect_submitted_assistant_prompts(buffer: &mut String, input: &str) -> Vec<String> {
    let normalized = normalize_assistant_input(input);
    let mut prompts = Vec::new();

    for character in normalized.chars() {
        match character {
            '\r' | '\n' => {
                let prompt = buffer.trim().to_string();
                if !prompt.is_empty() {
                    prompts.push(prompt);
                }
                buffer.clear();
            }
            '\u{0008}' | '\u{007f}' => {
                buffer.pop();
            }
            character if character >= ' ' => buffer.push(character),
            _ => {}
        }
    }

    prompts
}

#[cfg(test)]
mod tests {
    use super::{
        collect_submitted_assistant_prompts, normalize_assistant_input, parse_zmx_session,
        resolve_assistant_provider, AssistantProvider,
    };

    #[test]
    fn assistant_input_strips_escape_sequences() {
        assert_eq!(
            normalize_assistant_input("hello\u{001b}[31m world\u{001b}[0m"),
            "hello world"
        );
    }

    #[test]
    fn assistant_input_collects_prompts() {
        let mut buffer = String::new();
        assert_eq!(
            collect_submitted_assistant_prompts(&mut buffer, "hello world\r"),
            vec!["hello world".to_string()]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn zmx_session_parser_reads_name_and_command() {
        let session = parse_zmx_session("session_name=terminal-1\tpid=2\tcmd=codex")
            .expect("session should parse");
        assert_eq!(session.name, "terminal-1");
        assert_eq!(session.command.as_deref(), Some("codex"));
    }

    #[test]
    fn assistant_provider_resolution_handles_cc_alias() {
        assert_eq!(
            resolve_assistant_provider("cc"),
            Some(AssistantProvider::Claude)
        );
        assert_eq!(
            resolve_assistant_provider("codex"),
            Some(AssistantProvider::Codex)
        );
        assert_eq!(resolve_assistant_provider("bun run dev"), None);
    }
}
