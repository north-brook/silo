use std::collections::{BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

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
    pub(crate) git_ignored: bool,
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
    let workspace_root = workspace_root();
    list_workspace_files_in(&workspace_root)
}

fn list_workspace_files_in(workspace_root: &Path) -> Result<Vec<FileTreeEntry>, String> {
    let mut entries = Vec::new();
    collect_workspace_files(workspace_root, workspace_root, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.dedup_by(|left, right| left.path == right.path);
    mark_gitignored_entries(workspace_root, &mut entries)?;
    Ok(entries)
}

fn collect_workspace_files(
    workspace_root: &Path,
    directory: &Path,
    entries: &mut Vec<FileTreeEntry>,
) -> Result<(), String> {
    let read_dir = fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to read workspace directory {}: {error}",
            directory.display()
        )
    })?;

    for child in read_dir {
        let child = child.map_err(|error| {
            format!(
                "failed to read workspace directory {}: {error}",
                directory.display()
            )
        })?;
        let file_name = child.file_name();
        if directory == workspace_root && file_name == OsStr::new(".git") {
            continue;
        }

        let child_path = child.path();
        let file_type = child.file_type().map_err(|error| {
            format!(
                "failed to read workspace file type {}: {error}",
                child_path.display()
            )
        })?;

        if file_type.is_dir() {
            collect_workspace_files(workspace_root, &child_path, entries)?;
            continue;
        }

        if file_type.is_symlink() {
            match fs::metadata(&child_path) {
                Ok(metadata) if metadata.is_dir() => continue,
                Ok(_) | Err(_) => {}
            }
        } else if !file_type.is_file() {
            continue;
        }

        let path = child_path
            .strip_prefix(workspace_root)
            .map_err(|error| format!("failed to strip workspace root prefix: {error}"))?
            .to_str()
            .map(|value| value.replace('\\', "/"))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "file path must be valid UTF-8".to_string())?;
        entries.push(FileTreeEntry {
            path,
            git_ignored: false,
        });
    }

    Ok(())
}

fn mark_gitignored_entries(
    workspace_root: &Path,
    entries: &mut [FileTreeEntry],
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }

    let mut child = Command::new("git")
        .args(["-c", "core.quotepath=false", "check-ignore", "-z", "--stdin"])
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to determine gitignored files: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open git check-ignore stdin".to_string())?;
    for entry in &*entries {
        stdin
            .write_all(entry.path.as_bytes())
            .map_err(|error| format!("failed to write gitignored file list: {error}"))?;
        stdin
            .write_all(&[0])
            .map_err(|error| format!("failed to write gitignored file list: {error}"))?;
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|error| format!("failed to read gitignored file output: {error}"))?;

    match output.status.code() {
        Some(0) | Some(1) => {}
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "failed to determine gitignored files".to_string()
            } else {
                format!("failed to determine gitignored files: {stderr}")
            });
        }
    }

    let ignored_paths = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for entry in entries {
        entry.git_ignored = ignored_paths.contains(entry.path.as_str());
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn list_workspace_files_marks_gitignored_and_hidden_files() {
        let workspace = TestWorkspace::new("files-tree");
        workspace.write_file(".gitignore", "dist/\n*.env\nnode_modules/\n");
        workspace.write_file(".env", "secret");
        workspace.write_file("tracked.env", "tracked");
        workspace.write_file(".vscode/settings.json", "{}");
        workspace.write_file("dist/out.js", "console.log('build')");
        workspace.write_file("node_modules/pkg/index.js", "export {};");
        workspace.write_file("src/main.ts", "export const app = true;");
        workspace.git(&["add", ".gitignore", ".vscode/settings.json", "src/main.ts"]);
        workspace.git(&["add", "-f", "tracked.env"]);

        let entries =
            list_workspace_files_in(&workspace.root).expect("workspace files should list");

        assert_eq!(
            entries,
            vec![
                FileTreeEntry {
                    path: ".env".to_string(),
                    git_ignored: true,
                },
                FileTreeEntry {
                    path: ".gitignore".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: ".vscode/settings.json".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: "dist/out.js".to_string(),
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
                FileTreeEntry {
                    path: "tracked.env".to_string(),
                    git_ignored: false,
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_workspace_files_skips_git_dir_and_symlinked_directories() {
        let workspace = TestWorkspace::new("files-tree-symlink");
        workspace.write_file(".git/config", "[core]");
        workspace.write_file("linked-target/inner.txt", "inner");
        workspace.write_file("real.txt", "real");
        workspace.symlink("linked-target", "linked-dir");
        workspace.symlink("real.txt", "real-link.txt");

        let entries =
            list_workspace_files_in(&workspace.root).expect("workspace files should list");

        assert_eq!(
            entries,
            vec![
                FileTreeEntry {
                    path: "linked-target/inner.txt".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: "real-link.txt".to_string(),
                    git_ignored: false,
                },
                FileTreeEntry {
                    path: "real.txt".to_string(),
                    git_ignored: false,
                },
            ]
        );
        assert!(!entries.iter().any(|entry| entry.path.starts_with(".git/")));
        assert!(!entries.iter().any(|entry| entry.path.starts_with("linked-dir/")));
    }

    struct TestWorkspace {
        root: PathBuf,
    }

    impl TestWorkspace {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be available")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("workspace-agent-{name}-{unique}"));
            fs::create_dir_all(&root).expect("test workspace should create");
            let workspace = Self { root };
            workspace.git(&["init", "-q"]);
            workspace
        }

        fn write_file(&self, relative_path: &str, content: &str) {
            let path = self.root.join(relative_path);
            let parent = path.parent().expect("test file should have parent");
            fs::create_dir_all(parent).expect("test file parent should create");
            fs::write(path, content).expect("test file should write");
        }

        fn symlink(&self, target: &str, link: &str) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;

                symlink(self.root.join(target), self.root.join(link))
                    .expect("test symlink should create");
            }
        }

        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(args)
                .current_dir(&self.root)
                .output()
                .expect("git command should run");
            assert!(
                output.status.success(),
                "git {:?} should succeed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
