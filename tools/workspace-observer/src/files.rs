use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{self, Command};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::args::required_flag_value;
use crate::daemon::state::{FileWatchState, ObserverEvent};
use crate::runtime::{load_state, send_event, write_json_stdout, RuntimePaths};

const WORKSPACE_ROOT: &str = "/home/silo/workspace";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileTreeEntry {
    pub(crate) path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileReadResult {
    pub(crate) path: String,
    pub(crate) exists: bool,
    pub(crate) binary: bool,
    pub(crate) revision: String,
    pub(crate) content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FileWriteStatus {
    Saved,
    Conflict,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileWriteResult {
    pub(crate) status: FileWriteStatus,
    pub(crate) revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FileWatchEntry {
    pub(crate) path: String,
    pub(crate) exists: bool,
    pub(crate) binary: bool,
    pub(crate) revision: String,
}

pub(crate) fn run_files_tree() -> Result<(), String> {
    write_json_stdout(&list_workspace_files()?)
}

pub(crate) fn run_files_read(args: &[String]) -> Result<(), String> {
    let path = normalize_repo_relative_path(required_flag_value(args, "--path")?)?;
    write_json_stdout(&read_workspace_file(&path)?)
}

pub(crate) fn run_files_write(args: &[String]) -> Result<(), String> {
    let path = normalize_repo_relative_path(required_flag_value(args, "--path")?)?;
    let expected_revision = required_flag_value(args, "--expected-revision")?.trim();
    if expected_revision.is_empty() {
        return Err("file expected revision must not be empty".to_string());
    }

    let mut content = String::new();
    io::stdin()
        .read_to_string(&mut content)
        .map_err(|error| format!("failed to read file write stdin: {error}"))?;
    write_json_stdout(&write_workspace_file(&path, expected_revision, &content)?)
}

pub(crate) fn run_files_sync_watch_set() -> Result<(), String> {
    let mut payload = String::new();
    io::stdin()
        .read_to_string(&mut payload)
        .map_err(|error| format!("failed to read watch set stdin: {error}"))?;
    let paths = if payload.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str::<Vec<String>>(&payload)
            .map_err(|error| format!("invalid watch set json: {error}"))?
    };
    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_repo_relative_path(&path)?);
    }
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::FilesWatchSet {
            paths: normalized.into_iter().collect(),
        },
    )
}

pub(crate) fn run_files_watch_state() -> Result<(), String> {
    let state = load_state(&RuntimePaths::new().state_file).unwrap_or_default();
    let mut entries = state
        .files
        .watched
        .into_iter()
        .map(|(path, state)| FileWatchEntry {
            path,
            exists: state.exists,
            binary: state.binary,
            revision: state.revision,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    write_json_stdout(&entries)
}

pub(crate) fn list_workspace_files() -> Result<Vec<FileTreeEntry>, String> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.quotepath=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(workspace_root())
        .output()
        .map_err(|error| format!("failed to list workspace files: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "failed to list workspace files".to_string()
        } else {
            format!("failed to list workspace files: {stderr}")
        });
    }

    let mut entries = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|path| FileTreeEntry {
            path: path.to_string(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.dedup_by(|left, right| left.path == right.path);
    Ok(entries)
}

pub(crate) fn read_workspace_file(path: &str) -> Result<FileReadResult, String> {
    let normalized = normalize_repo_relative_path(path)?;
    let absolute_path = workspace_root().join(&normalized);
    if !absolute_path.exists() || absolute_path.is_dir() {
        return Ok(FileReadResult {
            path: normalized,
            exists: false,
            binary: false,
            revision: "missing".to_string(),
            content: None,
        });
    }

    let bytes = fs::read(&absolute_path)
        .map_err(|error| format!("failed to read workspace file {normalized}: {error}"))?;
    let revision = hex_sha256(&bytes);
    let binary = bytes.iter().any(|byte| *byte == 0) || std::str::from_utf8(&bytes).is_err();

    Ok(FileReadResult {
        path: normalized,
        exists: true,
        binary,
        revision,
        content: if binary {
            None
        } else {
            Some(
                String::from_utf8(bytes).map_err(|error| {
                    format!("failed to decode workspace file as utf-8: {error}")
                })?,
            )
        },
    })
}

pub(crate) fn write_workspace_file(
    path: &str,
    expected_revision: &str,
    content: &str,
) -> Result<FileWriteResult, String> {
    let normalized = normalize_repo_relative_path(path)?;
    let absolute_path = workspace_root().join(&normalized);
    if !absolute_path.exists() || absolute_path.is_dir() {
        return Ok(FileWriteResult {
            status: FileWriteStatus::Missing,
            revision: None,
        });
    }

    let current_bytes = fs::read(&absolute_path)
        .map_err(|error| format!("failed to read workspace file {normalized}: {error}"))?;
    let current_revision = hex_sha256(&current_bytes);
    if current_revision != expected_revision {
        return Ok(FileWriteResult {
            status: FileWriteStatus::Conflict,
            revision: Some(current_revision),
        });
    }

    let metadata = fs::metadata(&absolute_path)
        .map_err(|error| format!("failed to stat workspace file {normalized}: {error}"))?;
    let parent = absolute_path
        .parent()
        .ok_or_else(|| format!("workspace file {normalized} is missing a parent dir"))?;
    let temp_path = parent.join(format!(
        ".silo-write-{}-{}",
        process::id(),
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    fs::write(&temp_path, content.as_bytes())
        .map_err(|error| format!("failed to stage workspace file {normalized}: {error}"))?;
    fs::set_permissions(&temp_path, metadata.permissions()).map_err(|error| {
        format!("failed to copy workspace file permissions {normalized}: {error}")
    })?;
    fs::rename(&temp_path, &absolute_path)
        .map_err(|error| format!("failed to replace workspace file {normalized}: {error}"))?;

    Ok(FileWriteResult {
        status: FileWriteStatus::Saved,
        revision: Some(hex_sha256(content.as_bytes())),
    })
}

pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(WORKSPACE_ROOT)
}

pub(crate) fn normalize_repo_relative_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("file path must not be empty".to_string());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(trimmed).components() {
        match component {
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => {
                return Err("file path must stay within the workspace root".to_string());
            }
        }
    }

    normalized
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "file path must be valid UTF-8".to_string())
}

pub(crate) fn observed_file_state(path: &str) -> Result<FileWatchState, String> {
    let file = read_workspace_file(path)?;
    Ok(FileWatchState {
        exists: file.exists,
        binary: file.binary,
        revision: file.revision,
    })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
