use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::daemon::state::{ObserverEvent, ObserverState};

const FIFO_MODE: u32 = 0o622;

#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    pub(crate) root: PathBuf,
    pub(crate) fifo: PathBuf,
    pub(crate) pidfile: PathBuf,
    pub(crate) state_file: PathBuf,
}

impl RuntimePaths {
    pub(crate) fn new() -> Self {
        let root = PathBuf::from("/home/silo/.silo/workspace-agent");
        Self {
            fifo: root.join("events.fifo"),
            pidfile: root.join("daemon.pid"),
            state_file: root.join("state.json"),
            root,
        }
    }

    pub(crate) fn ensure(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|error| {
            format!(
                "failed to create agent runtime dir {}: {error}",
                self.root.display()
            )
        })?;
        fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to set runtime dir permissions: {error}"))
    }
}

pub(crate) fn load_state(path: &Path) -> Result<ObserverState, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read state file {}: {error}", path.display()))?;
    serde_json::from_str(&contents).map_err(|error| format!("invalid state json: {error}"))
}

pub(crate) fn persist_state(path: &Path, state: &ObserverState) -> Result<(), String> {
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

pub(crate) fn write_json_stdout<T: Serialize>(value: &T) -> Result<(), String> {
    let payload = serde_json::to_string(value).map_err(|error| error.to_string())?;
    println!("{payload}");
    Ok(())
}

pub(crate) fn ensure_fifo(path: &Path) -> Result<(), String> {
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

pub(crate) fn spawn_fifo_reader(path: PathBuf, tx: Sender<ObserverEvent>) {
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
            match reader.read_line(&mut line) {
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

pub(crate) fn send_event(path: &Path, event: &ObserverEvent) -> Result<(), String> {
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

pub(crate) fn acquire_pidfile(path: &Path) -> Result<bool, String> {
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
