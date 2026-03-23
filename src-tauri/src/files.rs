use crate::bootstrap;
use crate::remote::{
    run_remote_command, run_remote_command_with_stdin, shell_quote, workspace_shell_command,
    workspace_shell_command_preserving_stdin, REMOTE_WORKSPACE_AGENT_BIN,
};
use crate::state::{active_session_metadata_entries, WorkspaceMetadataManager};
use crate::workspaces::{self, WorkspaceLookup, WorkspaceSession};
use crate::{emit_workspace_state_changed, AppRuntime};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTreeEntry {
    path: String,
    git_ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileTreeNodeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTreeNode {
    path: String,
    name: String,
    kind: FileTreeNodeKind,
    git_ignored: bool,
    expandable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTreeDirectory {
    directory_path: String,
    entries: Vec<FileTreeNode>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceFileAsset {
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct WorkspaceFileAssetPayload {
    content_base64: Option<String>,
    status: String,
}

#[tauri::command]
pub async fn files_list_tree(workspace: String) -> Result<Vec<FileTreeEntry>, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    run_agent_json_command(
        &lookup,
        "failed to list workspace files",
        &agent_remote_command("files-tree"),
    )
    .await
}

#[tauri::command]
pub async fn files_list_directory(
    workspace: String,
    path: Option<String>,
) -> Result<FileTreeDirectory, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let directory_path = match path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(path) => Some(normalize_repo_relative_path(path)?),
        None => None,
    };
    let command = match directory_path.as_deref() {
        Some(path) => {
            agent_remote_command(&format!("files-directory --path {}", shell_quote(path)))
        }
        None => agent_remote_command("files-directory"),
    };

    match run_agent_json_command(&lookup, "failed to list workspace directory", &command).await {
        Ok(directory) => Ok(directory),
        Err(error) if is_unknown_files_directory_command_error(&error) => {
            let entries = run_agent_json_command::<Vec<FileTreeEntry>>(
                &lookup,
                "failed to list workspace files",
                &agent_remote_command("files-tree"),
            )
            .await?;
            Ok(file_tree_directory_from_legacy_entries(
                directory_path.as_deref().unwrap_or(""),
                &entries,
            ))
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn files_read(workspace: String, path: String) -> Result<FileReadResult, String> {
    let lookup = branch_workspace_lookup(&workspace).await?;
    let path = normalize_repo_relative_path(&path)?;
    run_agent_json_command(
        &lookup,
        "failed to read workspace file",
        &agent_remote_command(&format!("files-read --path {}", shell_quote(&path))),
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
    run_agent_json_command_with_stdin(
        &lookup,
        "failed to save workspace file",
        &agent_remote_command(&format!(
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
        &workspace_shell_command_preserving_stdin(&agent_remote_command("files-sync-watch-set")),
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
    run_agent_json_command(
        &lookup,
        "failed to read watched file state",
        &agent_remote_command("files-watch-state"),
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

    state
        .inner()
        .enqueue_workspace_session_upsert(&workspace, Some(lookup), session.clone());

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

    let cleared_active_session =
        state.clear_active_workspace_session_if_matches(&workspace, "file", &attachment_id, None);
    if cleared_active_session {
        state.enqueue(&workspace, None, active_session_metadata_entries(None));
    }

    state
        .inner()
        .enqueue_workspace_session_remove(&workspace, None, "file", &attachment_id);
    emit_workspace_state_changed(
        &app,
        &workspace,
        Some(("file", &attachment_id)),
        cleared_active_session,
        None,
    );
    Ok(())
}

pub(crate) async fn read_workspace_file_asset(
    workspace: &str,
    path: &str,
) -> Result<Option<WorkspaceFileAsset>, String> {
    let lookup = branch_workspace_lookup(workspace).await?;
    let path = normalize_repo_relative_path(path)?;
    if browser_renderable_content_type(&path).is_none() {
        return Err(format!(
            "workspace file {} does not support browser rendering",
            path
        ));
    }

    let absolute_path = format!("/home/silo/workspace/{path}");
    let command = workspace_shell_command(&format!(
        "file_path={file_path}\n\
if [ ! -f \"$file_path\" ]; then\n\
  printf '%s' '{{\"status\":\"missing\"}}'\n\
  exit 0\n\
fi\n\
printf '%s' '{{\"status\":\"ok\",\"content_base64\":\"'\n\
base64 \"$file_path\" | tr -d '\\n'\n\
printf '%s' '\"}}'\n",
        file_path = shell_quote(&absolute_path),
    ));
    let result = run_remote_command(&lookup, &command).await?;
    if !result.success {
        return Err(file_command_error(
            "failed to read workspace file for browser",
            &result.stderr,
        ));
    }

    let payload = parse_json_output::<WorkspaceFileAssetPayload>(
        "failed to parse workspace file browser payload",
        &result.stdout,
        &result.stderr,
    )?;
    match payload.status.as_str() {
        "missing" => Ok(None),
        "ok" => {
            let encoded = payload
                .content_base64
                .as_deref()
                .ok_or_else(|| "workspace file browser payload missing content".to_string())?;
            let bytes = BASE64_STANDARD.decode(encoded).map_err(|error| {
                format!("failed to decode workspace file browser payload: {error}")
            })?;
            Ok(Some(WorkspaceFileAsset { bytes }))
        }
        other => Err(format!(
            "workspace file browser payload returned unsupported status: {other}"
        )),
    }
}

pub(crate) fn browser_renderable_content_type(path: &str) -> Option<&'static str> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;

    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

pub(crate) async fn branch_workspace_lookup(workspace: &str) -> Result<WorkspaceLookup, String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if lookup.workspace.is_template() {
        return Err(format!(
            "workspace {} is a template workspace and does not support file editing",
            workspace
        ));
    }
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspaces::workspace_not_ready_error(&lookup.workspace));
    }
    Ok(lookup)
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

async fn run_agent_json_command<T: for<'de> Deserialize<'de>>(
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

async fn run_agent_json_command_with_stdin<T: for<'de> Deserialize<'de>>(
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
            format!("{context}: invalid empty agent response: {error}")
        } else if trimmed_stderr.is_empty() {
            format!("{context}: invalid agent response: {error}; stdout={trimmed_stdout}")
        } else {
            format!(
                "{context}: invalid agent response: {error}; stdout={trimmed_stdout}; stderr={trimmed_stderr}"
            )
        }
    })
}

fn is_unknown_files_directory_command_error(error: &str) -> bool {
    error.contains("unknown command: files-directory")
}

fn file_tree_directory_from_legacy_entries(
    directory_path: &str,
    entries: &[FileTreeEntry],
) -> FileTreeDirectory {
    let mut direct_entries = Vec::new();
    let mut directories = HashMap::<String, (String, bool)>::new();
    let directory_prefix = (!directory_path.is_empty()).then(|| format!("{directory_path}/"));

    for entry in entries {
        let remainder = match directory_prefix.as_deref() {
            Some(prefix) => match entry.path.strip_prefix(prefix) {
                Some(value) => value,
                None => continue,
            },
            None => entry.path.as_str(),
        };

        if remainder.is_empty() {
            continue;
        }

        if let Some((name, _)) = remainder.split_once('/') {
            let child_path = if directory_path.is_empty() {
                name.to_string()
            } else {
                format!("{directory_path}/{name}")
            };
            directories
                .entry(child_path)
                .and_modify(|(_, git_ignored)| *git_ignored &= entry.git_ignored)
                .or_insert_with(|| (name.to_string(), entry.git_ignored));
            continue;
        }

        direct_entries.push(FileTreeNode {
            path: entry.path.clone(),
            name: remainder.to_string(),
            kind: FileTreeNodeKind::File,
            git_ignored: entry.git_ignored,
            expandable: false,
        });
    }

    direct_entries.extend(directories.into_iter().map(|(path, (name, git_ignored))| {
        FileTreeNode {
            path,
            name,
            kind: FileTreeNodeKind::Directory,
            git_ignored,
            expandable: true,
        }
    }));
    sort_file_tree_nodes(&mut direct_entries);

    FileTreeDirectory {
        directory_path: directory_path.to_string(),
        entries: direct_entries,
    }
}

fn sort_file_tree_nodes(entries: &mut [FileTreeNode]) {
    entries.sort_by(|left, right| match (&left.kind, &right.kind) {
        (FileTreeNodeKind::Directory, FileTreeNodeKind::File) => std::cmp::Ordering::Less,
        (FileTreeNodeKind::File, FileTreeNodeKind::Directory) => std::cmp::Ordering::Greater,
        _ => left.name.cmp(&right.name),
    });
}

pub(crate) fn normalize_repo_relative_path(path: &str) -> Result<String, String> {
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

pub(crate) fn file_display_name(path: &str) -> String {
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

fn file_command_error(prefix: &str, stderr: &str) -> String {
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

    #[test]
    fn browser_renderable_content_type_matches_supported_files() {
        assert_eq!(
            browser_renderable_content_type("images/photo.PNG"),
            Some("image/png")
        );
        assert_eq!(
            browser_renderable_content_type("docs/manual.pdf"),
            Some("application/pdf")
        );
        assert_eq!(
            browser_renderable_content_type("icons/logo.svg"),
            Some("image/svg+xml")
        );
    }

    #[test]
    fn browser_renderable_content_type_rejects_unsupported_files() {
        assert_eq!(browser_renderable_content_type("archive.zip"), None);
        assert_eq!(browser_renderable_content_type("notes.txt"), None);
    }

    #[test]
    fn normalize_repo_relative_path_rejects_parent_segments() {
        assert_eq!(
            normalize_repo_relative_path("../secret.txt"),
            Err("file path must stay within the workspace root".to_string())
        );
    }

    #[test]
    fn file_tree_directory_from_legacy_entries_returns_root_children() {
        let directory = file_tree_directory_from_legacy_entries(
            "",
            &[
                FileTreeEntry {
                    path: "README.md".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: "src/main.ts".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: "node_modules/pkg/index.js".to_string(),
                    git_ignored: true,
                },
            ],
        );

        assert_eq!(
            directory,
            FileTreeDirectory {
                directory_path: "".to_string(),
                entries: vec![
                    FileTreeNode {
                        path: "node_modules".to_string(),
                        name: "node_modules".to_string(),
                        kind: FileTreeNodeKind::Directory,
                        git_ignored: true,
                        expandable: true,
                    },
                    FileTreeNode {
                        path: "src".to_string(),
                        name: "src".to_string(),
                        kind: FileTreeNodeKind::Directory,
                        git_ignored: false,
                        expandable: true,
                    },
                    FileTreeNode {
                        path: "README.md".to_string(),
                        name: "README.md".to_string(),
                        kind: FileTreeNodeKind::File,
                        git_ignored: false,
                        expandable: false,
                    },
                ],
            }
        );
    }

    #[test]
    fn file_tree_directory_from_legacy_entries_returns_nested_children() {
        let directory = file_tree_directory_from_legacy_entries(
            "node_modules",
            &[
                FileTreeEntry {
                    path: "node_modules/.bin/vite".to_string(),
                    git_ignored: true,
                },
                FileTreeEntry {
                    path: "node_modules/pkg/index.js".to_string(),
                    git_ignored: true,
                },
                FileTreeEntry {
                    path: "src/main.ts".to_string(),
                    git_ignored: false,
                },
            ],
        );

        assert_eq!(
            directory,
            FileTreeDirectory {
                directory_path: "node_modules".to_string(),
                entries: vec![
                    FileTreeNode {
                        path: "node_modules/.bin".to_string(),
                        name: ".bin".to_string(),
                        kind: FileTreeNodeKind::Directory,
                        git_ignored: true,
                        expandable: true,
                    },
                    FileTreeNode {
                        path: "node_modules/pkg".to_string(),
                        name: "pkg".to_string(),
                        kind: FileTreeNodeKind::Directory,
                        git_ignored: true,
                        expandable: true,
                    },
                ],
            }
        );
    }
}
