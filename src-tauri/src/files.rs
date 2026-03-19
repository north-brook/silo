use crate::remote::{
    run_remote_command, run_remote_command_with_stdin, shell_quote, workspace_shell_command,
    workspace_shell_command_preserving_stdin, REMOTE_WORKSPACE_OBSERVER_BIN,
};
use crate::state::{
    active_session_metadata_entries, file_session_metadata_key, WorkspaceMetadataEntry,
    WorkspaceMetadataManager,
};
use crate::workspaces::{self, WorkspaceLookup, WorkspaceSession};
use crate::{emit_workspace_state_changed, AppRuntime};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTreeEntry {
    path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileReadResult {
    path: String,
    exists: bool,
    binary: bool,
    revision: String,
    content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileSaveStatus {
    Saved,
    Conflict,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSaveResult {
    status: FileSaveStatus,
    revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileSessionResult {
    attachment_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchedFileState {
    path: String,
    exists: bool,
    binary: bool,
    revision: String,
}

#[tauri::command]
pub async fn files_list_tree(workspace: String) -> Result<Vec<FileTreeEntry>, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    run_observer_json_command(
        &lookup,
        "failed to list workspace files",
        &observer_remote_command("files-tree"),
    )
    .await
}

#[tauri::command]
pub async fn files_read(workspace: String, path: String) -> Result<FileReadResult, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let path = normalize_repo_relative_path(&path)?;
    run_observer_json_command(
        &lookup,
        "failed to read workspace file",
        &observer_remote_command(&format!("files-read --path {}", shell_quote(&path))),
    )
    .await
}

#[tauri::command]
pub async fn files_save(
    workspace: String,
    path: String,
    content: String,
    base_revision: String,
) -> Result<FileSaveResult, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let path = normalize_repo_relative_path(&path)?;
    let base_revision = normalize_revision(&base_revision)?;
    run_observer_json_command_with_stdin(
        &lookup,
        "failed to save workspace file",
        &observer_remote_command(&format!(
            "files-write --path {} --expected-revision {}",
            shell_quote(&path),
            shell_quote(&base_revision),
        )),
        content.into_bytes(),
    )
    .await
}

#[tauri::command]
pub async fn files_set_watched_paths(workspace: String, paths: Vec<String>) -> Result<(), String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let mut normalized = paths
        .iter()
        .map(|path| normalize_repo_relative_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();

    let result = run_remote_command_with_stdin(
        &lookup,
        &workspace_shell_command_preserving_stdin(&observer_remote_command("files-sync-watch-set")),
        serde_json::to_vec(&normalized).map_err(|error| error.to_string())?,
    )
    .await?;
    if !result.success {
        return Err(file_command_error(
            "failed to sync watched file paths",
            &result.stderr,
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn files_get_watched_state(workspace: String) -> Result<Vec<WatchedFileState>, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    run_observer_json_command(
        &lookup,
        "failed to read watched file state",
        &observer_remote_command("files-watch-state"),
    )
    .await
}

#[tauri::command]
pub async fn files_open_session(
    state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    path: String,
) -> Result<FileSessionResult, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let path = normalize_repo_relative_path(&path)?;
    let workspace_state = state.apply_workspace_state(lookup.workspace.clone());

    if let Some(existing) = workspace_state
        .files()
        .iter()
        .find(|session| session.path.as_deref() == Some(path.as_str()))
    {
        return Ok(FileSessionResult {
            attachment_id: existing.attachment_id.clone(),
        });
    }

    let existing_names = workspace_state
        .files()
        .iter()
        .map(|session| session.attachment_id.clone())
        .collect::<HashSet<_>>();
    let session = WorkspaceSession {
        kind: "file".to_string(),
        name: file_display_name(&path),
        attachment_id: generate_file_attachment_id(&existing_names),
        path: Some(path),
        url: None,
        logical_url: None,
        resolved_url: None,
        title: None,
        favicon_url: None,
        can_go_back: None,
        can_go_forward: None,
        working: None,
        unread: None,
    };

    enqueue_file_metadata_update(state.inner(), &workspace, Some(lookup), session.clone());

    Ok(FileSessionResult {
        attachment_id: session.attachment_id,
    })
}

#[tauri::command]
pub fn files_close_session(
    app: AppHandle<AppRuntime>,
    state: State<'_, WorkspaceMetadataManager>,
    workspace: String,
    attachment_id: String,
) -> Result<(), String> {
    let attachment_id = attachment_id.trim().to_string();
    if attachment_id.is_empty() {
        return Err("file attachment_id must not be empty".to_string());
    }

    if state.clear_active_workspace_session_if_matches(
        &workspace,
        "file",
        &attachment_id,
        None,
    ) {
        state.enqueue(
            &workspace,
            None,
            active_session_metadata_entries(None),
        );
    }

    enqueue_file_metadata_remove(state.inner(), &workspace, None, &attachment_id);
    emit_workspace_state_changed(&app, &workspace);
    Ok(())
}

async fn branch_workspace_lookup(workspace: &str) -> Result<WorkspaceLookup, String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if lookup.workspace.is_template() {
        return Err(format!(
            "workspace {} is a template workspace and does not support file editing",
            workspace
        ));
    }
    if !lookup.workspace.ready() {
        return Err(format!("workspace {workspace} is not ready"));
    }
    Ok(lookup)
}

fn observer_remote_command(command: &str) -> String {
    format!(
        "if [ ! -x {observer_bin} ]; then\n\
  echo 'workspace-observer is unavailable' >&2\n\
  exit 1\n\
fi\n\
{observer_bin} {command}",
        observer_bin = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN),
    )
}

async fn run_observer_json_command<T: for<'de> Deserialize<'de>>(
    lookup: &WorkspaceLookup,
    context: &str,
    command: &str,
) -> Result<T, String> {
    let result = run_remote_command(lookup, &workspace_shell_command(command)).await?;
    if !result.success {
        return Err(file_command_error(context, &result.stderr));
    }
    parse_json_output(context, &result.stdout, &result.stderr)
}

async fn run_observer_json_command_with_stdin<T: for<'de> Deserialize<'de>>(
    lookup: &WorkspaceLookup,
    context: &str,
    command: &str,
    stdin_bytes: Vec<u8>,
) -> Result<T, String> {
    let result = run_remote_command_with_stdin(
        lookup,
        &workspace_shell_command_preserving_stdin(command),
        stdin_bytes,
    )
    .await?;
    if !result.success {
        return Err(file_command_error(context, &result.stderr));
    }
    parse_json_output(context, &result.stdout, &result.stderr)
}

fn parse_json_output<T: for<'de> Deserialize<'de>>(
    context: &str,
    stdout: &str,
    stderr: &str,
) -> Result<T, String> {
    serde_json::from_str(stdout).map_err(|error| {
        let trimmed_stdout = stdout.trim();
        let trimmed_stderr = stderr.trim();
        if trimmed_stdout.is_empty() && trimmed_stderr.is_empty() {
            format!("{context}: invalid empty observer response: {error}")
        } else if trimmed_stderr.is_empty() {
            format!("{context}: invalid observer response: {error}; stdout={trimmed_stdout}")
        } else {
            format!(
                "{context}: invalid observer response: {error}; stdout={trimmed_stdout}; stderr={trimmed_stderr}"
            )
        }
    })
}

fn normalize_repo_relative_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("file path must not be empty".to_string());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                return Err("file path must stay within the workspace root".to_string());
            }
        }
    }

    let normalized = normalized
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| "file path must be valid UTF-8".to_string())?;
    if normalized.is_empty() {
        return Err("file path must not be empty".to_string());
    }

    Ok(normalized)
}

fn normalize_revision(revision: &str) -> Result<String, String> {
    let trimmed = revision.trim();
    if trimmed.is_empty() {
        return Err("file base revision must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn file_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

fn generate_file_attachment_id(existing_names: &HashSet<String>) -> String {
    let mut timestamp = current_unix_timestamp_millis();
    loop {
        let candidate = format!("file-{timestamp}");
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

fn enqueue_file_metadata_update(
    metadata: &WorkspaceMetadataManager,
    workspace: &str,
    lookup: Option<WorkspaceLookup>,
    session: WorkspaceSession,
) {
    let serialized = match serde_json::to_string(&session) {
        Ok(serialized) => serialized,
        Err(error) => {
            log::warn!(
                "failed to serialize file session metadata for workspace {} session {}: {}",
                workspace,
                session.attachment_id,
                error
            );
            return;
        }
    };
    metadata.upsert_workspace_session(workspace, session.clone());
    metadata.enqueue(
        workspace,
        lookup,
        vec![
            WorkspaceMetadataEntry {
                key: file_session_metadata_key(&session.attachment_id),
                value: Some(serialized),
            },
            WorkspaceMetadataEntry {
                key: "file-last-active".to_string(),
                value: Some(current_rfc3339_timestamp()),
            },
        ],
    );
}

fn enqueue_file_metadata_remove(
    metadata: &WorkspaceMetadataManager,
    workspace: &str,
    lookup: Option<WorkspaceLookup>,
    attachment_id: &str,
) {
    metadata.remove_workspace_session(workspace, "file", attachment_id);
    metadata.enqueue(
        workspace,
        lookup,
        vec![WorkspaceMetadataEntry {
            key: file_session_metadata_key(attachment_id),
            value: None,
        }],
    );
}

fn current_rfc3339_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn file_command_error(prefix: &str, stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {trimmed}")
    }
}
