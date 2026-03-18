use crate::workspaces::WorkspaceLookup;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::io::Write;
use std::process::{Command, Output, Stdio};

const TERMINAL_USER: &str = "silo";
pub(crate) const TERMINAL_WORKSPACE_DIR: &str = "/home/silo/workspace";
const REMOTE_CREDENTIALS_FILE: &str = "/home/silo/.silo/credentials.sh";
pub(crate) const REMOTE_WORKSPACE_OBSERVER_BIN: &str = "/home/silo/.silo/bin/workspace-observer";

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
    run_gcloud_ssh_command(lookup, Some(remote_command.to_string()), None).await
}

pub(crate) async fn run_remote_command_with_stdin(
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

pub(crate) fn command_result_with_stdin_write_error(
    output: Output,
    error: &std::io::Error,
) -> CommandResult {
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
}
