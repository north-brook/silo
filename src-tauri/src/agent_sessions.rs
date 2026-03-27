use crate::remote::{
    run_remote_command, run_remote_command_with_stdin, shell_quote, workspace_shell_command,
    workspace_shell_command_preserving_stdin, REMOTE_WORKSPACE_AGENT_BIN,
};
use crate::workspaces::{WorkspaceActiveSession, WorkspaceLookup, WorkspaceSession};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct WorkspaceSessionSnapshot {
    #[serde(default)]
    pub(crate) working: bool,
    #[serde(default)]
    pub(crate) unread: bool,
    #[serde(default)]
    pub(crate) last_active: Option<String>,
    #[serde(default)]
    pub(crate) last_working: Option<String>,
    #[serde(default)]
    pub(crate) active_session: Option<WorkspaceActiveSession>,
    #[serde(default)]
    pub(crate) terminals: Vec<WorkspaceSession>,
    #[serde(default)]
    pub(crate) browsers: Vec<WorkspaceSession>,
    #[serde(default)]
    pub(crate) files: Vec<WorkspaceSession>,
}

pub(crate) async fn fetch_session_snapshot(
    lookup: &WorkspaceLookup,
) -> Result<WorkspaceSessionSnapshot, String> {
    if !lookup.workspace.is_ready() {
        return Ok(WorkspaceSessionSnapshot::default());
    }

    run_agent_json_command(
        lookup,
        "failed to read workspace sessions snapshot",
        &agent_remote_command("sessions-snapshot"),
    )
    .await
}

pub(crate) async fn upsert_session(
    lookup: &WorkspaceLookup,
    session: &WorkspaceSession,
) -> Result<(), String> {
    run_agent_command_with_stdin(
        lookup,
        "failed to persist workspace session",
        &agent_remote_command("session-upsert"),
        serde_json::to_vec(session).map_err(|error| error.to_string())?,
    )
    .await
}

pub(crate) async fn remove_session(
    lookup: &WorkspaceLookup,
    kind: &str,
    attachment_id: &str,
) -> Result<(), String> {
    run_agent_command(
        lookup,
        "failed to remove workspace session",
        &agent_remote_command(&format!(
            "session-remove --type {} --attachment-id {}",
            shell_quote(kind),
            shell_quote(attachment_id),
        )),
    )
    .await
}

pub(crate) async fn set_active_session(
    lookup: &WorkspaceLookup,
    active_session: Option<&WorkspaceActiveSession>,
) -> Result<(), String> {
    let command = match active_session {
        Some(active_session) => agent_remote_command(&format!(
            "session-set-active --type {} --attachment-id {}",
            shell_quote(&active_session.kind),
            shell_quote(&active_session.attachment_id),
        )),
        None => agent_remote_command("session-clear-active"),
    };
    run_agent_command(
        lookup,
        "failed to update active workspace session",
        &command,
    )
    .await
}

fn agent_remote_command(command: &str) -> String {
    format!(
        "if [ ! -x {agent_bin} ]; then\n\
  echo 'workspace-agent is unavailable' >&2\n\
  exit 1\n\
fi\n\
{agent_bin} {command}",
        agent_bin = shell_quote(REMOTE_WORKSPACE_AGENT_BIN),
    )
}

async fn run_agent_command(
    lookup: &WorkspaceLookup,
    context: &str,
    command: &str,
) -> Result<(), String> {
    let result = run_remote_command(lookup, &workspace_shell_command(command)).await?;
    if result.success {
        return Ok(());
    }
    Err(agent_command_error(context, &result.stderr))
}

async fn run_agent_command_with_stdin(
    lookup: &WorkspaceLookup,
    context: &str,
    command: &str,
    stdin_bytes: Vec<u8>,
) -> Result<(), String> {
    let result = run_remote_command_with_stdin(
        lookup,
        &workspace_shell_command_preserving_stdin(command),
        stdin_bytes,
    )
    .await?;
    if result.success {
        return Ok(());
    }
    Err(agent_command_error(context, &result.stderr))
}

async fn run_agent_json_command<T: for<'de> Deserialize<'de>>(
    lookup: &WorkspaceLookup,
    context: &str,
    command: &str,
) -> Result<T, String> {
    let result = run_remote_command(lookup, &workspace_shell_command(command)).await?;
    if !result.success {
        return Err(agent_command_error(context, &result.stderr));
    }
    serde_json::from_str(&result.stdout).map_err(|error| {
        let stdout = result.stdout.trim();
        let stderr = result.stderr.trim();
        if stdout.is_empty() && stderr.is_empty() {
            format!("{context}: invalid empty agent response: {error}")
        } else if stderr.is_empty() {
            format!("{context}: invalid agent response: {error}; stdout={stdout}")
        } else {
            format!("{context}: invalid agent response: {error}; stdout={stdout}; stderr={stderr}")
        }
    })
}

fn agent_command_error(context: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {stderr}")
    }
}
