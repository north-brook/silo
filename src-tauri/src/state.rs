use crate::workspaces::{self, Workspace, WorkspaceLookup, WorkspaceSession};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceMetadataEntry {
    pub(crate) key: String,
    pub(crate) value: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingWorkspaceMetadata {
    lookup: Option<WorkspaceLookup>,
    entries: HashMap<String, Option<String>>,
    worker_running: bool,
}

#[derive(Clone, Default)]
pub struct WorkspaceMetadataManager {
    metadata: Arc<Mutex<HashMap<String, PendingWorkspaceMetadata>>>,
    sessions: Arc<Mutex<HashMap<String, HashMap<String, Option<WorkspaceSession>>>>>,
}

const WORKSPACE_METADATA_FLUSH_DELAY: Duration = Duration::from_millis(40);
const WORKSPACE_METADATA_BACKGROUND_RETRY_ATTEMPTS: usize = 2;
const WORKSPACE_METADATA_BACKGROUND_RETRY_INTERVAL: Duration = Duration::from_millis(100);
pub(crate) const BROWSER_LAST_ACTIVE_METADATA_KEY: &str = "browser-last-active";
pub(crate) const BROWSER_SESSION_METADATA_PREFIX: &str = "browser-session-";
pub(crate) const TERMINAL_LAST_ACTIVE_METADATA_KEY: &str = "terminal-last-active";
pub(crate) const TERMINAL_LAST_WORKING_METADATA_KEY: &str = "terminal-last-working";
pub(crate) const TERMINAL_SESSION_METADATA_PREFIX: &str = "terminal-session-";
pub(crate) const TERMINAL_UNREAD_METADATA_KEY: &str = "terminal-unread";
pub(crate) const TERMINAL_WORKING_METADATA_KEY: &str = "terminal-working";

pub(crate) fn browser_session_metadata_key(attachment_id: &str) -> String {
    format!("{BROWSER_SESSION_METADATA_PREFIX}{attachment_id}")
}

fn workspace_session_key(kind: &str, attachment_id: &str) -> String {
    format!("{kind}:{attachment_id}")
}

fn should_drop_pending_session(
    pending: &WorkspaceSession,
    metadata: Option<&WorkspaceSession>,
) -> bool {
    match metadata {
        Some(metadata_session) if metadata_session == pending => true,
        Some(_) => {
            pending.kind == "terminal"
                && pending.name == "shell"
                && pending.working.is_none()
                && pending.unread.is_none()
        }
        None => false,
    }
}

impl WorkspaceMetadataManager {
    pub(crate) fn enqueue(
        &self,
        workspace: &str,
        lookup: Option<WorkspaceLookup>,
        entries: Vec<WorkspaceMetadataEntry>,
    ) {
        if entries.is_empty() {
            return;
        }

        let workspace_name = workspace.to_string();
        let mut should_spawn = false;
        if let Ok(mut pending) = self.metadata.lock() {
            let state =
                pending
                    .entry(workspace_name.clone())
                    .or_insert_with(|| PendingWorkspaceMetadata {
                        lookup: None,
                        entries: HashMap::new(),
                        worker_running: false,
                    });
            if let Some(lookup) = lookup {
                state.lookup = Some(lookup);
            }
            for entry in entries {
                state.entries.insert(entry.key, entry.value);
            }
            if !state.worker_running {
                state.worker_running = true;
                should_spawn = true;
            }
        }

        if !should_spawn {
            return;
        }

        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.process_workspace_queue(workspace_name).await;
        });
    }

    async fn process_workspace_queue(&self, workspace: String) {
        loop {
            sleep_for(WORKSPACE_METADATA_FLUSH_DELAY).await;

            let (lookup, entries) = {
                let mut pending = match self.metadata.lock() {
                    Ok(pending) => pending,
                    Err(_) => return,
                };
                let Some(state) = pending.get_mut(&workspace) else {
                    return;
                };
                if state.entries.is_empty() {
                    state.worker_running = false;
                    pending.remove(&workspace);
                    return;
                }

                let lookup = state.lookup.clone();
                let entries = state
                    .entries
                    .drain()
                    .map(|(key, value)| WorkspaceMetadataEntry { key, value })
                    .collect::<Vec<_>>();
                (lookup, entries)
            };

            let mut current_lookup = match lookup {
                Some(lookup) => lookup,
                None => match workspaces::find_workspace(&workspace).await {
                    Ok(lookup) => lookup,
                    Err(error) => {
                        log::warn!(
                            "failed to resolve workspace {} for background metadata update: {}",
                            workspace,
                            error
                        );
                        continue;
                    }
                },
            };

            let mut update_result = Err("workspace metadata update did not run".to_string());
            for attempt in 0..WORKSPACE_METADATA_BACKGROUND_RETRY_ATTEMPTS {
                update_result = workspaces::apply_workspace_metadata_entries_in_lookup(
                    current_lookup.clone(),
                    &entries,
                )
                .await;
                if update_result.is_ok() {
                    break;
                }
                if attempt + 1 < WORKSPACE_METADATA_BACKGROUND_RETRY_ATTEMPTS {
                    sleep_for(WORKSPACE_METADATA_BACKGROUND_RETRY_INTERVAL).await;
                    if let Ok(refreshed) = workspaces::find_workspace(&workspace).await {
                        current_lookup = refreshed;
                    }
                }
            }

            if let Err(error) = update_result {
                log::warn!(
                    "background metadata update failed for workspace {} keys=[{}]: {}",
                    workspace,
                    entries
                        .iter()
                        .map(|entry| entry.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    error
                );
            } else if let Ok(mut pending) = self.metadata.lock() {
                if let Some(state) = pending.get_mut(&workspace) {
                    state.lookup = Some(current_lookup);
                }
            }
        }
    }

    pub(crate) fn upsert_workspace_session(&self, workspace: &str, session: WorkspaceSession) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        sessions
            .entry(workspace.to_string())
            .or_default()
            .insert(
                workspace_session_key(&session.kind, &session.attachment_id),
                Some(session),
            );
    }

    pub(crate) fn mark_workspace_session_read(
        &self,
        workspace: &str,
        attachment_id: &str,
        session: Option<WorkspaceSession>,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let entries = sessions.entry(workspace.to_string()).or_default();
        let key = workspace_session_key("terminal", attachment_id);
        let next = entries
            .get(&key)
            .and_then(|existing| existing.clone())
            .or(session)
            .map(|mut session| {
                session.unread = Some(false);
                session
            });
        if let Some(session) = next {
            entries.insert(key, Some(session));
        }
    }

    pub(crate) fn remove_workspace_session(
        &self,
        workspace: &str,
        kind: &str,
        attachment_id: &str,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        sessions
            .entry(workspace.to_string())
            .or_default()
            .insert(workspace_session_key(kind, attachment_id), None);
    }

    pub(crate) fn apply_workspace_state(&self, workspace: Workspace) -> Workspace {
        let workspace_name = workspace.name().to_string();
        let metadata_sessions = workspace
            .sessions()
            .into_iter()
            .map(|session| {
                (
                    workspace_session_key(&session.kind, &session.attachment_id),
                    session,
                )
            })
            .collect::<HashMap<_, _>>();

        let overlay = {
            let Ok(mut sessions) = self.sessions.lock() else {
                return workspace;
            };
            let Some(entries) = sessions.get_mut(&workspace_name) else {
                return workspace;
            };
            entries.retain(|key, pending| match pending {
                Some(session) => !should_drop_pending_session(session, metadata_sessions.get(key)),
                None => metadata_sessions.contains_key(key),
            });
            if entries.is_empty() {
                sessions.remove(&workspace_name);
                return workspace;
            }
            entries.clone()
        };

        workspaces::overlay_workspace_sessions(workspace, &overlay)
    }

    pub(crate) fn apply_workspace_states(&self, workspaces: Vec<Workspace>) -> Vec<Workspace> {
        workspaces
            .into_iter()
            .map(|workspace| self.apply_workspace_state(workspace))
            .collect()
    }
}

async fn sleep_for(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}
