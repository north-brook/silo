use std::env;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{self, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;

use crate::args::{optional_flag_value, required_flag_value};
use crate::daemon::state::{AssistantProvider, ObserverEvent};
use crate::runtime::{send_event, RuntimePaths};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) fn run_assistant_proxy(args: &[String]) -> Result<(), String> {
    let provider = required_flag_value(args, "--provider")?;
    let provider = AssistantProvider::parse(provider)
        .ok_or_else(|| format!("unsupported assistant provider: {provider}"))?;
    let command = build_wrapped_command(args)?;

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
    builder.env("SILO_TERMINAL_ID", &session);
    builder.env("SILO_ASSISTANT_PROVIDER", provider.command_name());
    builder.env(
        "SILO_WORKSPACE_AGENT_BIN",
        assistant_agent_bin_path().as_deref().unwrap_or_default(),
    );
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

    send_event(
        &runtime.fifo,
        &ObserverEvent::AssistantSessionStarted {
            session: session.clone(),
            provider,
        },
    )?;
    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_done_signal = Arc::clone(&reader_done);
    let reader_thread = thread::spawn(move || {
        proxy_output(reader, io::stdout());
        reader_done_signal.store(true, Ordering::Relaxed);
    });
    let resize_stop = Arc::new(AtomicBool::new(false));
    let resize_thread = spawn_resize_forwarder(Arc::clone(&master), Arc::clone(&resize_stop));

    let raw_mode = RawModeGuard::new().map_err(|error| error.to_string())?;
    let _input_thread = thread::spawn(move || {
        let _ = proxy_input(io::stdin(), writer);
    });
    let status = child
        .wait()
        .map_err(|error| format!("assistant command wait failed: {error}"))?;
    drop(raw_mode);
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

fn build_wrapped_command(args: &[String]) -> Result<Vec<String>, String> {
    let command_start = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or_else(|| "assistant-proxy requires `--` before the wrapped command".to_string())?;
    let initial_prompt_argv = args[..command_start]
        .iter()
        .any(|arg| arg == "--initial-prompt-argv");
    let initial_prompt_file = optional_flag_value(&args[..command_start], "--initial-prompt-file")?;
    if initial_prompt_argv && initial_prompt_file.is_some() {
        return Err("assistant-proxy accepts only one initial prompt source".to_string());
    }

    let mut command = args[command_start + 1..].to_vec();
    if command.is_empty() {
        return Err("assistant-proxy requires a wrapped command".to_string());
    }

    if let Some(path) = initial_prompt_file {
        let prompt = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read initial prompt file: {error}"))?;
        let _ = std::fs::remove_file(path);
        command.push(prompt);
    }

    Ok(command)
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

fn proxy_input<R: Read>(mut stdin: R, mut writer: Box<dyn Write + Send>) -> Result<(), String> {
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
    }
}

fn proxy_output<R: Read, W: Write>(mut reader: R, mut stdout: W) {
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

pub(crate) fn run_assistant_hook(args: &[String]) -> Result<(), String> {
    let provider = required_flag_value(args, "--provider")?;
    let provider = AssistantProvider::parse(provider)
        .ok_or_else(|| format!("unsupported assistant provider: {provider}"))?;
    let session = match env::var("SILO_TERMINAL_ID") {
        Ok(session) if !session.trim().is_empty() => session,
        _ => return Ok(()),
    };

    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        return Ok(());
    }

    let Some(event) = observer_event_from_hook_input(&session, provider, &input) else {
        return Ok(());
    };

    let _ = send_event(&RuntimePaths::new().fifo, &event);
    Ok(())
}

pub(crate) fn observer_event_from_hook_input(
    session: &str,
    provider: AssistantProvider,
    input: &str,
) -> Option<ObserverEvent> {
    let hook = serde_json::from_str::<AssistantHookInput>(input).ok()?;
    let turn_id = hook.turn_id.and_then(non_empty_string);
    match hook.hook_event_name.as_str() {
        "UserPromptSubmit" => Some(ObserverEvent::AssistantPromptSubmitted {
            session: session.to_string(),
            provider,
            turn_id,
        }),
        "Stop" => Some(ObserverEvent::AssistantTurnCompleted {
            session: session.to_string(),
            provider,
            turn_id,
        }),
        _ => None,
    }
}

fn assistant_agent_bin_path() -> Option<String> {
    env::var("SILO_WORKSPACE_AGENT_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::current_exe()
                .ok()
                .map(|path| path.display().to_string())
        })
}

#[derive(Debug, Deserialize)]
struct AssistantHookInput {
    #[serde(default, alias = "hookEventName")]
    hook_event_name: String,
    #[serde(default, alias = "turnId")]
    turn_id: Option<String>,
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn build_wrapped_command_keeps_prompt_argv_passthrough() {
        let command = build_wrapped_command(&proxy_args(&[
            "--provider",
            "codex",
            "--initial-prompt-argv",
            "--",
            "codex",
            "ship it",
        ]))
        .expect("argv prompt command should parse");

        assert_eq!(command, vec!["codex".to_string(), "ship it".to_string()]);
    }

    #[test]
    fn build_wrapped_command_appends_prompt_from_file() {
        let prompt_path =
            std::env::temp_dir().join(format!("assistant-proxy-prompt-{}.txt", std::process::id()));
        std::fs::write(&prompt_path, "ship it\nexplain why").expect("prompt file should write");

        let command = build_wrapped_command(&proxy_args(&[
            "--provider",
            "codex",
            "--initial-prompt-file",
            prompt_path.to_str().expect("prompt path should be utf8"),
            "--",
            "codex",
        ]))
        .expect("file prompt command should parse");

        assert_eq!(
            command,
            vec!["codex".to_string(), "ship it\nexplain why".to_string()]
        );
        assert!(!prompt_path.exists());
    }

    #[test]
    fn build_wrapped_command_rejects_multiple_prompt_sources() {
        let error = build_wrapped_command(&proxy_args(&[
            "--provider",
            "codex",
            "--initial-prompt-argv",
            "--initial-prompt-file",
            "/tmp/prompt.txt",
            "--",
            "codex",
        ]))
        .expect_err("multiple prompt sources should fail");

        assert_eq!(
            error,
            "assistant-proxy accepts only one initial prompt source"
        );
    }
}
