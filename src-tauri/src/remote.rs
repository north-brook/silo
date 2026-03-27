use crate::gcp;
use crate::workspaces::WorkspaceLookup;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use portable_pty::CommandBuilder;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

const TERMINAL_USER: &str = "silo";
pub(crate) const TERMINAL_WORKSPACE_DIR: &str = "/home/silo/workspace";
const REMOTE_CREDENTIALS_FILE: &str = "/home/silo/.silo/credentials.sh";
pub(crate) const REMOTE_WORKSPACE_AGENT_BIN: &str = "/home/silo/.silo/bin/workspace-agent";
const SSH_CONTROL_PERSIST: &str = "600";
const SSH_SESSION_REFRESH_BEFORE: Duration = Duration::from_secs(30);
const SSH_SESSION_FAILURE_RETRY_AFTER: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
struct SshSession {
    username: String,
    host: String,
    key_path: PathBuf,
    known_hosts_path: PathBuf,
    control_path: PathBuf,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct CachedSshFailure {
    error: String,
    retry_after: Instant,
}

static SSH_SESSIONS: OnceLock<Mutex<HashMap<String, SshSession>>> = OnceLock::new();
static SSH_SESSION_FAILURES: OnceLock<Mutex<HashMap<String, CachedSshFailure>>> = OnceLock::new();
static SSH_SESSION_GATES: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct CommandResult {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn assistant_prompt_command(prefix: &str, prompt: &str) -> String {
    let encoded_prompt = BASE64_STANDARD.encode(prompt);
    format!(
        "{prefix} \"$(printf %s {} | base64 --decode)\"",
        shell_quote(&encoded_prompt)
    )
}

pub(crate) fn terminal_shell_command(command: &str) -> String {
    format!(
        "if [ -f {credentials_path} ]; then source {credentials_path}; fi; cd {workspace_dir}; {command}",
        credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE),
        workspace_dir = shell_quote(TERMINAL_WORKSPACE_DIR),
        command = command,
    )
}

pub(crate) async fn run_remote_command(
    lookup: &WorkspaceLookup,
    remote_command: &str,
) -> Result<CommandResult, String> {
    run_ssh_command(lookup, remote_command.to_string(), None).await
}

pub(crate) async fn run_remote_command_with_stdin(
    lookup: &WorkspaceLookup,
    remote_command: &str,
    stdin_bytes: Vec<u8>,
) -> Result<CommandResult, String> {
    run_ssh_command(lookup, remote_command.to_string(), Some(stdin_bytes)).await
}

fn ssh_sessions() -> &'static Mutex<HashMap<String, SshSession>> {
    SSH_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ssh_session_failures() -> &'static Mutex<HashMap<String, CachedSshFailure>> {
    SSH_SESSION_FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn ssh_session_gates() -> &'static Mutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    SSH_SESSION_GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session_key(lookup: &WorkspaceLookup) -> String {
    format!(
        "{}:{}:{}",
        lookup.gcloud_project,
        lookup.workspace.zone(),
        lookup.workspace.name()
    )
}

fn session_gate(cache_key: &str) -> Result<Arc<AsyncMutex<()>>, String> {
    let mut guard = ssh_session_gates()
        .lock()
        .map_err(|_| "ssh session gate cache lock poisoned".to_string())?;
    Ok(guard
        .entry(cache_key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone())
}

fn cached_session_failure(cache_key: &str) -> Result<Option<String>, String> {
    let mut guard = ssh_session_failures()
        .lock()
        .map_err(|_| "ssh session failure cache lock poisoned".to_string())?;
    if let Some(cached) = guard.get(cache_key) {
        if Instant::now() < cached.retry_after {
            return Ok(Some(cached.error.clone()));
        }
    }
    guard.remove(cache_key);
    Ok(None)
}

fn store_session_failure(cache_key: &str, error: &str) -> Result<(), String> {
    ssh_session_failures()
        .lock()
        .map_err(|_| "ssh session failure cache lock poisoned".to_string())?
        .insert(
            cache_key.to_string(),
            CachedSshFailure {
                error: error.to_string(),
                retry_after: Instant::now() + SSH_SESSION_FAILURE_RETRY_AFTER,
            },
        );
    Ok(())
}

fn clear_session_failure(cache_key: &str) -> Result<(), String> {
    ssh_session_failures()
        .lock()
        .map_err(|_| "ssh session failure cache lock poisoned".to_string())?
        .remove(cache_key);
    Ok(())
}

fn ssh_destination(session: &SshSession) -> String {
    format!("{}@{}", session.username, session.host)
}

fn ssh_base_args(session: &SshSession) -> Vec<String> {
    vec![
        "-i".to_string(),
        session.key_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        "IdentitiesOnly=yes".to_string(),
        "-o".to_string(),
        format!(
            "UserKnownHostsFile={}",
            session.known_hosts_path.to_string_lossy()
        ),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-o".to_string(),
        "LogLevel=ERROR".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
    ]
}

fn ssh_common_args(session: &SshSession) -> Vec<String> {
    let mut args = ssh_base_args(session);
    args.extend([
        "-o".to_string(),
        format!("ControlPath={}", session.control_path.to_string_lossy()),
        "-o".to_string(),
        format!("ControlPersist={SSH_CONTROL_PERSIST}"),
    ]);
    args
}

fn ssh_port_forward_args(session: &SshSession, local_port: u16, remote_port: u16) -> Vec<String> {
    let mut args = ssh_base_args(session);
    args.extend([
        "-o".to_string(),
        "ControlMaster=no".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-N".to_string(),
        "-L".to_string(),
        format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"),
        ssh_destination(session),
    ]);
    args
}

fn start_master_connection(session: &SshSession) -> Result<(), String> {
    if let Some(parent) = session.control_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create ssh control directory: {error}"))?;
    }
    let _ = std::fs::remove_file(&session.control_path);
    let mut command = Command::new("ssh");
    command.args(ssh_common_args(session));
    command.arg("-M");
    command.arg("-f");
    command.arg("-N");
    command.arg(ssh_destination(session));
    command.env("SSH_AUTH_SOCK", "");
    let output = command
        .output()
        .map_err(|error| format!("failed to start ssh control master: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(remote_command_error(
            "failed to start ssh control master",
            &String::from_utf8_lossy(&output.stderr),
        ))
    }
}

fn is_ssh_publickey_error(error: &str) -> bool {
    error.contains("Permission denied (publickey)")
}

fn is_ssh_host_key_mismatch_error(error: &str) -> bool {
    error.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || error.contains("Host key verification failed")
}

fn known_host_entry_matches_host(line: &str, host: &str) -> bool {
    let Some(hosts) = line.split_whitespace().next() else {
        return false;
    };
    if hosts.starts_with('#') {
        return false;
    }
    let port_host = format!("[{host}]:22");
    hosts
        .split(',')
        .any(|candidate| candidate == host || candidate == port_host)
}

fn remove_known_host_entry(path: &Path, host: &str) -> Result<bool, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "failed to read ssh known hosts file {}: {error}",
                path.display()
            ));
        }
    };

    let mut removed = false;
    let mut retained = Vec::new();
    for line in contents.lines() {
        if known_host_entry_matches_host(line, host) {
            removed = true;
            continue;
        }
        retained.push(line);
    }
    if !removed {
        return Ok(false);
    }

    let mut updated = retained.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    std::fs::write(path, updated).map_err(|error| {
        format!(
            "failed to update ssh known hosts file {}: {error}",
            path.display()
        )
    })?;
    Ok(true)
}

fn control_master_alive(session: &SshSession) -> bool {
    if !session.control_path.exists() {
        return false;
    }
    let mut command = Command::new("ssh");
    command.args(ssh_common_args(session));
    command.arg("-O");
    command.arg("check");
    command.arg(ssh_destination(session));
    command.env("SSH_AUTH_SOCK", "");
    command
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

async fn build_ssh_session(lookup: &WorkspaceLookup) -> Result<SshSession, String> {
    let endpoint = gcp::instance_endpoint(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        lookup.workspace.name(),
    )
    .await?;
    let login = gcp::prepare_oslogin_session(
        &lookup.gcloud_project,
        &endpoint.zone,
        lookup.workspace.name(),
    )
    .await?;
    let session = SshSession {
        username: login.username,
        host: endpoint.host,
        key_path: login.key_path,
        known_hosts_path: gcp::ssh_known_hosts_path()?,
        control_path: gcp::ssh_control_path(
            &lookup.gcloud_project,
            lookup.workspace.zone(),
            lookup.workspace.name(),
        )?,
        expires_at: Instant::now() + Duration::from_secs(600),
    };
    tauri::async_runtime::spawn_blocking({
        let session = session.clone();
        move || start_master_connection(&session)
    })
    .await
    .map_err(|error| format!("ssh control master task failed: {error}"))??;
    Ok(session)
}

async fn invalidate_cached_workspace_host_key(lookup: &WorkspaceLookup) -> Result<bool, String> {
    let endpoint = gcp::instance_endpoint(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        lookup.workspace.name(),
    )
    .await?;
    let known_hosts_path = gcp::ssh_known_hosts_path()?;
    tauri::async_runtime::spawn_blocking(move || {
        remove_known_host_entry(&known_hosts_path, &endpoint.host)
    })
    .await
    .map_err(|error| format!("ssh known hosts cleanup task failed: {error}"))?
}

async fn ensure_ssh_session(lookup: &WorkspaceLookup) -> Result<SshSession, String> {
    let cache_key = session_key(lookup);
    let gate = session_gate(&cache_key)?;
    if let Some(existing) = reusable_ssh_session(&cache_key).await? {
        return Ok(existing);
    }
    if let Some(error) = cached_session_failure(&cache_key)? {
        return Err(error);
    }
    let _guard = gate.lock().await;
    if let Some(existing) = reusable_ssh_session(&cache_key).await? {
        return Ok(existing);
    }
    if let Some(error) = cached_session_failure(&cache_key)? {
        return Err(error);
    }

    let mut result = build_ssh_session(lookup).await;
    if let Err(error) = &result {
        if is_ssh_publickey_error(error) {
            match gcp::invalidate_cached_oslogin_ssh_key(
                &lookup.gcloud_project,
                lookup.workspace.zone(),
                lookup.workspace.name(),
            ) {
                Ok(invalidated) => {
                    log::warn!(
                        "ssh publickey auth failed while establishing session workspace={} invalidated_cached_oslogin_key={} retrying=true",
                        lookup.workspace.name(),
                        invalidated
                    );
                    result = build_ssh_session(lookup).await;
                }
                Err(invalidate_error) => {
                    log::warn!(
                        "ssh publickey auth failed while establishing session workspace={} but cached OS Login key invalidation failed: {}",
                        lookup.workspace.name(),
                        invalidate_error
                    );
                }
            }
        } else if is_ssh_host_key_mismatch_error(error) {
            match invalidate_cached_workspace_host_key(lookup).await {
                Ok(invalidated) => {
                    log::warn!(
                        "ssh host key mismatch while establishing session workspace={} invalidated_cached_host_key={} retrying=true",
                        lookup.workspace.name(),
                        invalidated
                    );
                    result = build_ssh_session(lookup).await;
                }
                Err(invalidate_error) => {
                    log::warn!(
                        "ssh host key mismatch while establishing session workspace={} but known hosts invalidation failed: {}",
                        lookup.workspace.name(),
                        invalidate_error
                    );
                }
            }
        }
    }

    match result {
        Ok(session) => {
            clear_session_failure(&cache_key)?;
            ssh_sessions()
                .lock()
                .map_err(|_| "ssh session cache lock poisoned".to_string())?
                .insert(cache_key, session.clone());
            Ok(session)
        }
        Err(error) => {
            store_session_failure(&cache_key, &error)?;
            Err(error)
        }
    }
}

async fn reusable_ssh_session(cache_key: &str) -> Result<Option<SshSession>, String> {
    let existing = {
        let guard = ssh_sessions()
            .lock()
            .map_err(|_| "ssh session cache lock poisoned".to_string())?;
        guard.get(cache_key).cloned()
    };
    let Some(existing) = existing else {
        return Ok(None);
    };
    let reusable = Instant::now() + SSH_SESSION_REFRESH_BEFORE < existing.expires_at
        && tauri::async_runtime::spawn_blocking({
            let existing = existing.clone();
            move || control_master_alive(&existing)
        })
        .await
        .map_err(|error| format!("ssh control check task failed: {error}"))?;
    if reusable {
        Ok(Some(existing))
    } else {
        Ok(None)
    }
}

fn ensure_ssh_session_blocking(lookup: &WorkspaceLookup) -> Result<SshSession, String> {
    tauri::async_runtime::block_on(ensure_ssh_session(lookup))
}

async fn run_ssh_command(
    lookup: &WorkspaceLookup,
    remote_command: String,
    stdin_bytes: Option<Vec<u8>>,
) -> Result<CommandResult, String> {
    let session = ensure_ssh_session(lookup).await?;
    let destination = ssh_destination(&session);
    let mut args = ssh_common_args(&session);
    args.push("-T".to_string());
    args.push(destination);
    args.push(remote_command);

    tauri::async_runtime::spawn_blocking(move || {
        let mut command = Command::new("ssh");
        command.args(&args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin_bytes.is_some() {
            command.stdin(Stdio::piped());
        }
        command.env("SSH_AUTH_SOCK", "");

        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to execute ssh: {error}"))?;

        if let Some(stdin_bytes) = stdin_bytes {
            if let Some(mut stdin) = child.stdin.take() {
                if let Err(error) = stdin.write_all(&stdin_bytes) {
                    drop(stdin);
                    let output = child
                        .wait_with_output()
                        .map_err(|wait_error| format!("failed to read ssh output: {wait_error}"))?;
                    return Ok(command_result_with_stdin_write_error(output, &error));
                }
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|error| format!("failed to read ssh output: {error}"))?;

        Ok(command_result_from_output(output))
    })
    .await
    .map_err(|error| format!("ssh task failed: {error}"))?
}

fn command_result_from_output(output: Output) -> CommandResult {
    CommandResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub(crate) fn command_result_with_stdin_write_error(
    output: Output,
    error: &std::io::Error,
) -> CommandResult {
    let mut result = command_result_from_output(output);
    if result.success {
        return result;
    }
    let write_error = format!("failed to write ssh stdin: {error}");
    result.stderr = if result.stderr.trim().is_empty() {
        write_error
    } else {
        format!("{}\n{write_error}", result.stderr.trim_end())
    };
    result
}

pub(crate) fn spawn_remote_port_forward(
    lookup: &WorkspaceLookup,
    local_port: u16,
    remote_port: u16,
) -> Result<Child, String> {
    let session = ensure_ssh_session_blocking(lookup)?;
    let mut command = Command::new("ssh");
    command.args(ssh_port_forward_args(&session, local_port, remote_port));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.env("SSH_AUTH_SOCK", "");
    command
        .spawn()
        .map_err(|error| format!("failed to start ssh port forward: {error}"))
}

pub(crate) async fn build_terminal_attach_command(
    lookup: &WorkspaceLookup,
    remote_command: &str,
) -> Result<CommandBuilder, String> {
    let session = ensure_ssh_session(lookup).await?;
    let mut command = CommandBuilder::new("ssh");
    command.args(ssh_common_args(&session));
    command.args([
        "-tt".to_string(),
        ssh_destination(&session),
        wrap_remote_shell_command(remote_command),
    ]);
    Ok(command)
}

pub(crate) fn wrap_remote_shell_command(command: &str) -> String {
    command.to_string()
}

pub(crate) fn run_terminal_user_command(command: &str) -> String {
    format!("sudo -iu {TERMINAL_USER} bash -lc {}", shell_quote(command))
}

pub(crate) fn workspace_shell_command(command: &str) -> String {
    workspace_shell_command_with_prelude(None, command)
}

pub(crate) fn workspace_shell_command_preserving_stdin(command: &str) -> String {
    workspace_shell_command_preserving_stdin_with_prelude(None, command)
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

fn workspace_shell_command_preserving_stdin_with_prelude(
    prelude: Option<&str>,
    command: &str,
) -> String {
    let script = workspace_shell_script(prelude, command);
    let encoded = BASE64_STANDARD.encode(script);
    run_terminal_user_command(&format!(
        "exec bash -lc \"$(printf %s {} | base64 --decode)\"",
        shell_quote(&encoded)
    ))
}

pub(crate) fn workspace_shell_script(prelude: Option<&str>, command: &str) -> String {
    format!(
        "set -euo pipefail\nexport LC_ALL=C\nexport LANG=C\n{}\ncd {}\n{}",
        prelude.unwrap_or_default(),
        shell_quote(TERMINAL_WORKSPACE_DIR),
        command
    )
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn remote_command_error(prefix: &str, stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(unix)]
    use std::process::{ExitStatus, Output};

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
    fn workspace_shell_command_preserving_stdin_wraps_script_via_base64() {
        let command = workspace_shell_command_preserving_stdin("printf \"hi\\n\"");
        assert!(command.starts_with("sudo -iu silo bash -lc 'exec bash -lc \"$(printf %s "));
        assert!(command.contains("| base64 --decode)\"'"));
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
    fn ssh_port_forward_args_use_dedicated_connection() {
        let session = SshSession {
            username: "svc_user".to_string(),
            host: "203.0.113.10".to_string(),
            key_path: PathBuf::from("/tmp/test-key"),
            known_hosts_path: PathBuf::from("/tmp/test-known-hosts"),
            control_path: PathBuf::from("/tmp/test-control"),
            expires_at: Instant::now(),
        };

        let args = ssh_port_forward_args(&session, 43123, 3000);
        let joined = args.join(" ");

        assert!(joined.contains("ControlMaster=no"));
        assert!(joined.contains("ExitOnForwardFailure=yes"));
        assert!(!joined.contains("ControlPath=/tmp/test-control"));
        assert!(!joined.contains("ControlPersist=600"));
        assert!(joined.contains("127.0.0.1:43123:127.0.0.1:3000"));
        assert!(joined.ends_with("svc_user@203.0.113.10"));
    }

    #[cfg(unix)]
    #[test]
    fn stdin_write_error_preserves_remote_stderr_and_forces_failure() {
        let output = Output {
            status: ExitStatus::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"ssh: connect to host 1.2.3.4 port 22: Connection refused\n".to_vec(),
        };
        let error = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe");

        let result = command_result_with_stdin_write_error(output, &error);

        assert!(!result.success);
        assert!(result.stderr.contains("Connection refused"));
        assert!(result.stderr.contains("Broken pipe"));
    }

    #[cfg(unix)]
    #[test]
    fn stdin_write_error_preserves_successful_remote_exit() {
        let output = Output {
            status: ExitStatus::from_raw(0),
            stdout: b"done\n".to_vec(),
            stderr: b"bootstrap state already up to date\n".to_vec(),
        };
        let error = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe");

        let result = command_result_with_stdin_write_error(output, &error);

        assert!(result.success);
        assert_eq!(result.stdout, "done\n");
        assert_eq!(result.stderr, "bootstrap state already up to date\n");
        assert!(!result.stderr.contains("Broken pipe"));
    }

    #[test]
    fn cached_session_failure_expires_after_retry_window() {
        let key = "demo:us-east4-c:workspace";
        {
            let mut guard = ssh_session_failures().lock().unwrap();
            guard.insert(
                key.to_string(),
                CachedSshFailure {
                    error: "boom".to_string(),
                    retry_after: Instant::now() - Duration::from_secs(1),
                },
            );
        }

        assert_eq!(cached_session_failure(key).unwrap(), None);
        assert!(ssh_session_failures().lock().unwrap().get(key).is_none());
    }

    #[test]
    fn ssh_publickey_error_matches_auth_failures() {
        assert!(is_ssh_publickey_error(
            "failed to start ssh control master: user@host: Permission denied (publickey)."
        ));
        assert!(!is_ssh_publickey_error(
            "failed to start ssh control master: user@host: Connection timed out"
        ));
    }

    #[test]
    fn ssh_host_key_mismatch_error_matches_changed_host_failures() {
        assert!(is_ssh_host_key_mismatch_error(
            "failed to start ssh control master: @@@@@@@@@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@@@@@@@@@"
        ));
        assert!(is_ssh_host_key_mismatch_error(
            "failed to start ssh control master: Host key verification failed."
        ));
        assert!(!is_ssh_host_key_mismatch_error(
            "failed to start ssh control master: ssh: connect to host 1.2.3.4 port 22: Connection timed out"
        ));
    }

    #[test]
    fn remove_known_host_entry_removes_matching_host_lines() {
        let path = std::env::temp_dir().join(format!("known-hosts-{}", uuid::Uuid::new_v4()));
        std::fs::write(
            &path,
            "\
35.245.191.100 ssh-ed25519 old-a
136.107.138.114 ssh-ed25519 old-b
[136.107.138.114]:22 ssh-ed25519 old-c
",
        )
        .unwrap();

        let removed = remove_known_host_entry(&path, "136.107.138.114").unwrap();
        let updated = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(removed);
        assert_eq!(updated, "35.245.191.100 ssh-ed25519 old-a\n");
    }
}
