use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, BufRead};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::daemon::state::{ObserverEvent, ObserverState};

const FIFO_MODE: u32 = 0o622;
const SEND_EVENT_OPEN_ATTEMPTS: usize = 10;
const SEND_EVENT_OPEN_RETRY_DELAY: Duration = Duration::from_millis(100);

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
        let (file, _keepalive_writer) = match open_fifo_reader(&path) {
            Ok(handles) => handles,
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

fn open_fifo_reader(path: &Path) -> io::Result<(File, File)> {
    let path_cstring = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

    let read_fd = unsafe { libc::open(path_cstring.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if read_fd == -1 {
        return Err(io::Error::last_os_error());
    }

    let keepalive_fd =
        unsafe { libc::open(path_cstring.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
    if keepalive_fd == -1 {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(read_fd);
        }
        return Err(error);
    }

    if let Err(error) = clear_nonblocking(read_fd) {
        unsafe {
            libc::close(read_fd);
            libc::close(keepalive_fd);
        }
        return Err(error);
    }

    let reader = unsafe { File::from_raw_fd(read_fd) };
    let keepalive = unsafe { File::from_raw_fd(keepalive_fd) };
    Ok((reader, keepalive))
}

fn clear_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }

    let result = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

pub(crate) fn send_event(path: &Path, event: &ObserverEvent) -> Result<(), String> {
    let payload = serde_json::to_string(event).map_err(|error| error.to_string())?;
    ensure_fifo(path)?;
    let path_cstring = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|error| format!("invalid fifo path: {error}"))?;
    let fd = open_fifo_writer_with_retry(path, &path_cstring)?;

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

fn open_fifo_writer_with_retry(path: &Path, path_cstring: &CString) -> Result<i32, String> {
    for attempt in 0..SEND_EVENT_OPEN_ATTEMPTS {
        let fd = unsafe { libc::open(path_cstring.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd != -1 {
            return Ok(fd);
        }

        let error = io::Error::last_os_error();
        if attempt + 1 < SEND_EVENT_OPEN_ATTEMPTS && should_retry_fifo_open(&error) {
            thread::sleep(SEND_EVENT_OPEN_RETRY_DELAY);
            continue;
        }

        return Err(format!(
            "failed to open event fifo {}: {}",
            path.display(),
            error
        ));
    }

    Err(format!(
        "failed to open event fifo {} after {} attempts",
        path.display(),
        SEND_EVENT_OPEN_ATTEMPTS
    ))
}

fn should_retry_fifo_open(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == libc::ENXIO || code == libc::ENOENT || code == libc::EINTR
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::fs::FileTypeExt;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fifo_reader_allows_nonblocking_writers() {
        let path = fifo_test_path("events");
        create_fifo(&path);

        let (reader, _keepalive) = open_fifo_reader(&path).expect("reader should open");
        assert!(path
            .metadata()
            .expect("fifo metadata")
            .file_type()
            .is_fifo());

        send_event(
            &path,
            &ObserverEvent::MarkRead {
                session: "demo".to_string(),
            },
        )
        .expect("writer should connect");

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("reader should receive payload");
        assert!(line.contains("\"MarkRead\""));
        assert!(line.contains("\"demo\""));

        fs::remove_file(&path).expect("cleanup fifo");
    }

    #[test]
    fn send_event_retries_until_reader_is_available() {
        let path = fifo_test_path("retry");
        create_fifo(&path);

        let reader_path = path.clone();
        let reader_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            let (reader, _keepalive) =
                open_fifo_reader(&reader_path).expect("reader should open after retry");
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("reader should receive payload");
            line
        });

        send_event(
            &path,
            &ObserverEvent::MarkRead {
                session: "retry".to_string(),
            },
        )
        .expect("writer should retry until a reader is present");

        let line = reader_thread.join().expect("reader thread should finish");
        assert!(line.contains("\"retry\""));

        fs::remove_file(&path).expect("cleanup fifo");
    }

    fn fifo_test_path(suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be usable in tests")
            .as_nanos();
        std::env::temp_dir().join(format!("workspace-agent-{suffix}-{unique}.fifo"))
    }

    fn create_fifo(path: &Path) {
        let path_cstring =
            CString::new(path.to_string_lossy().as_bytes()).expect("fifo path should be valid");
        let result = unsafe { libc::mkfifo(path_cstring.as_ptr(), FIFO_MODE as libc::mode_t) };
        assert_eq!(
            result,
            0,
            "mkfifo should succeed: {}",
            io::Error::last_os_error()
        );
    }
}
