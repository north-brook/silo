use std::env;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{self, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use crate::args::required_flag_value;
use crate::daemon::state::{AssistantProvider, ObserverEvent};
use crate::runtime::{send_event, RuntimePaths};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const TURN_OUTPUT_IDLE_TIMEOUT: Duration = Duration::from_secs(6);
pub(crate) const INITIAL_PROMPT_STARTUP_GRACE: Duration = Duration::from_secs(6);
pub(crate) const SOFT_NEWLINE_SENTINEL: char = '\u{e000}';

pub(crate) fn run_assistant_proxy(args: &[String]) -> Result<(), String> {
    let provider = required_flag_value(args, "--provider")?;
    let provider = AssistantProvider::parse(provider)
        .ok_or_else(|| format!("unsupported assistant provider: {provider}"))?;
    let command_start = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or_else(|| "assistant-proxy requires `--` before the wrapped command".to_string())?;
    let initial_prompt_argv = args[..command_start]
        .iter()
        .any(|arg| arg == "--initial-prompt-argv");
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
        .openpty(pty_size(cols, rows))
        .map_err(|error| format!("failed to open pty: {error}"))?;

    let mut builder = CommandBuilder::new(&command[0]);
    if command.len() > 1 {
        builder.args(&command[1..]);
    }
    let cwd =
        env::current_dir().map_err(|error| format!("failed to read current directory: {error}"))?;
    builder.cwd(&cwd);
    builder.env("PWD", &cwd);
    builder.env("ZMX_SESSION", &session);
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(|error| format!("failed to spawn assistant command: {error}"))?;
    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));
    let reader = master
        .lock()
        .map_err(|_| "assistant pty lock poisoned".to_string())?
        .try_clone_reader()
        .map_err(|error| format!("failed to open pty reader: {error}"))?;
    let writer = master
        .lock()
        .map_err(|_| "assistant pty lock poisoned".to_string())?
        .take_writer()
        .map_err(|error| format!("failed to open pty writer: {error}"))?;

    let tracker = AssistantTracker::new(session.clone(), provider, runtime.fifo);
    send_event(
        &tracker.fifo,
        &ObserverEvent::AssistantSessionStarted {
            session: session.clone(),
            provider,
        },
    )?;
    if initial_prompt_argv
        && command
            .iter()
            .last()
            .is_some_and(|arg| !arg.trim().is_empty())
    {
        tracker.record_initial_prompt();
    }
    let reader_tracker = Arc::clone(&tracker);
    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_done_signal = Arc::clone(&reader_done);
    let reader_thread = thread::spawn(move || {
        proxy_output(reader, io::stdout(), reader_tracker);
        reader_done_signal.store(true, Ordering::Relaxed);
    });
    let resize_stop = Arc::new(AtomicBool::new(false));
    let resize_thread = spawn_resize_forwarder(Arc::clone(&master), Arc::clone(&resize_stop));

    let raw_mode = RawModeGuard::new().map_err(|error| error.to_string())?;
    let input_tracker = Arc::clone(&tracker);
    let _input_thread = thread::spawn(move || {
        let _ = proxy_input(io::stdin(), writer, input_tracker);
    });
    let status = child
        .wait()
        .map_err(|error| format!("assistant command wait failed: {error}"))?;
    drop(raw_mode);
    tracker.finish_turn_if_needed();
    tracker.stop();
    resize_stop.store(true, Ordering::Relaxed);

    while !reader_done.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
    let _ = reader_thread.join();
    let _ = resize_thread.join();

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

fn spawn_resize_forwarder(
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_size = current_terminal_size();
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(RESIZE_POLL_INTERVAL);
            let next_size = current_terminal_size();
            if next_size == last_size {
                continue;
            }

            if let Ok(master) = master.lock() {
                let _ = master.resize(pty_size(next_size.0, next_size.1));
            } else {
                return;
            }
            last_size = next_size;
        }
    })
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
    initial_prompt_argv_turn: bool,
    saw_output_for_turn: bool,
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
            collect_submitted_assistant_prompts(&mut state.input_buffer, input)
        };

        if prompts.is_empty() {
            return;
        }

        self.record_prompt_submission(false);
    }

    fn record_initial_prompt(&self) {
        self.record_prompt_submission(true);
    }

    fn record_output(&self, count: usize) {
        if count == 0 {
            return;
        }

        let mut state = self.state.lock().expect("tracker lock should not poison");
        if !state.awaiting_turn {
            return;
        }
        let timeout =
            turn_output_timeout(state.initial_prompt_argv_turn, state.saw_output_for_turn);
        state.saw_output_for_turn = true;
        state.deadline = Some(Instant::now() + timeout);
        self.wake.notify_all();
    }

    fn record_prompt_submission(&self, initial_prompt_argv_turn: bool) {
        {
            let mut state = self.state.lock().expect("tracker lock should not poison");
            state.awaiting_turn = true;
            state.deadline = None;
            state.initial_prompt_argv_turn = initial_prompt_argv_turn;
            state.saw_output_for_turn = false;
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

    fn finish_turn_if_needed(&self) {
        let should_emit = {
            let mut state = self.state.lock().expect("tracker lock should not poison");
            if !state.awaiting_turn {
                false
            } else {
                state.awaiting_turn = false;
                state.input_buffer.clear();
                state.deadline = None;
                state.initial_prompt_argv_turn = false;
                state.saw_output_for_turn = false;
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
            state.initial_prompt_argv_turn = false;
            state.saw_output_for_turn = false;
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

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}

pub(crate) fn turn_output_timeout(
    initial_prompt_argv_turn: bool,
    saw_output_for_turn: bool,
) -> Duration {
    if !saw_output_for_turn && initial_prompt_argv_turn {
        TURN_OUTPUT_IDLE_TIMEOUT + INITIAL_PROMPT_STARTUP_GRACE
    } else {
        TURN_OUTPUT_IDLE_TIMEOUT
    }
}

pub(crate) fn normalize_assistant_input(input: &str) -> String {
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
            let params_start = index + 2;
            index = params_start;
            while index < chars.len() {
                let control = chars[index];
                if ('@'..='~').contains(&control) {
                    let params = chars[params_start..index].iter().collect::<String>();
                    if is_soft_newline_escape(&params, control) {
                        output.push(SOFT_NEWLINE_SENTINEL);
                    }
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

pub(crate) fn collect_submitted_assistant_prompts(buffer: &mut String, input: &str) -> Vec<String> {
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
            SOFT_NEWLINE_SENTINEL => buffer.push('\n'),
            '\u{0008}' | '\u{007f}' => {
                buffer.pop();
            }
            character if character >= ' ' => buffer.push(character),
            _ => {}
        }
    }

    prompts
}

fn is_soft_newline_escape(params: &str, final_char: char) -> bool {
    matches!((params, final_char), ("13;2", 'u') | ("27;2;13", '~'))
}
