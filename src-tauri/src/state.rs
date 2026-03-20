use crate::templates::{TemplateOperationStatus, TemplateState};
use crate::workspaces::{
    self, Workspace, WorkspaceActiveSession, WorkspaceLookup, WorkspaceSession,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone)]
struct TransientTemplateState {
    state: TemplateState,
    cached_at: Instant,
}

#[derive(Clone, Default)]
pub struct WorkspaceMetadataManager {
    metadata: Arc<Mutex<HashMap<String, PendingWorkspaceMetadata>>>,
    sessions: Arc<Mutex<HashMap<String, HashMap<String, Option<WorkspaceSession>>>>>,
    active_sessions: Arc<Mutex<HashMap<String, Option<WorkspaceActiveSession>>>>,
    template_states: Arc<Mutex<HashMap<String, TransientTemplateState>>>,
    template_reconciles: Arc<Mutex<HashMap<String, bool>>>,
}

const WORKSPACE_METADATA_FLUSH_DELAY: Duration = Duration::from_millis(40);
const WORKSPACE_METADATA_BACKGROUND_RETRY_ATTEMPTS: usize = 2;
const WORKSPACE_METADATA_BACKGROUND_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const TEMPLATE_TRANSIENT_STATE_TTL: Duration = Duration::from_secs(6);
pub(crate) const BROWSER_LAST_ACTIVE_METADATA_KEY: &str = "browser-last-active";
pub(crate) const BROWSER_SESSION_METADATA_PREFIX: &str = "browser-session-";
pub(crate) const FILE_LAST_ACTIVE_METADATA_KEY: &str = "file-last-active";
pub(crate) const FILE_SESSION_METADATA_PREFIX: &str = "file-session-";
pub(crate) const ACTIVE_SESSION_KIND_METADATA_KEY: &str = "active-session-kind";
pub(crate) const ACTIVE_SESSION_ATTACHMENT_ID_METADATA_KEY: &str = "active-session-attachment-id";
pub(crate) const TERMINAL_LAST_ACTIVE_METADATA_KEY: &str = "terminal-last-active";
pub(crate) const TERMINAL_LAST_WORKING_METADATA_KEY: &str = "terminal-last-working";
pub(crate) const TERMINAL_SESSION_METADATA_PREFIX: &str = "terminal-session-";
pub(crate) const TERMINAL_UNREAD_METADATA_KEY: &str = "terminal-unread";
pub(crate) const TERMINAL_WORKING_METADATA_KEY: &str = "terminal-working";

pub(crate) fn browser_session_metadata_key(attachment_id: &str) -> String {
    format!("{BROWSER_SESSION_METADATA_PREFIX}{attachment_id}")
}

pub(crate) fn file_session_metadata_key(attachment_id: &str) -> String {
    format!("{FILE_SESSION_METADATA_PREFIX}{attachment_id}")
}

pub(crate) fn terminal_session_metadata_key(attachment_id: &str) -> String {
    format!("{TERMINAL_SESSION_METADATA_PREFIX}{attachment_id}")
}

pub(crate) fn active_session_metadata_entries(
    active_session: Option<&WorkspaceActiveSession>,
) -> Vec<WorkspaceMetadataEntry> {
    vec![
        WorkspaceMetadataEntry {
            key: ACTIVE_SESSION_KIND_METADATA_KEY.to_string(),
            value: active_session.map(|session| session.kind.clone()),
        },
        WorkspaceMetadataEntry {
            key: ACTIVE_SESSION_ATTACHMENT_ID_METADATA_KEY.to_string(),
            value: active_session.map(|session| session.attachment_id.clone()),
        },
    ]
}

pub(crate) fn workspace_last_active_metadata_key(kind: &str) -> Option<&'static str> {
    match kind {
        "browser" => Some(BROWSER_LAST_ACTIVE_METADATA_KEY),
        "file" => Some(FILE_LAST_ACTIVE_METADATA_KEY),
        "terminal" => Some(TERMINAL_LAST_ACTIVE_METADATA_KEY),
        _ => None,
    }
}

fn workspace_session_key(kind: &str, attachment_id: &str) -> String {
    format!("{kind}:{attachment_id}")
}

fn workspace_session_metadata_key(kind: &str, attachment_id: &str) -> Option<String> {
    match kind {
        "browser" => Some(browser_session_metadata_key(attachment_id)),
        "file" => Some(file_session_metadata_key(attachment_id)),
        "terminal" => Some(terminal_session_metadata_key(attachment_id)),
        _ => None,
    }
}

pub(crate) fn workspace_session_metadata_entries(
    workspace: &str,
    session: &WorkspaceSession,
) -> Option<Vec<WorkspaceMetadataEntry>> {
    let session_key = match workspace_session_metadata_key(&session.kind, &session.attachment_id) {
        Some(key) => key,
        None => {
            log::warn!(
                "unsupported workspace session metadata kind {} for workspace {} session {}",
                session.kind,
                workspace,
                session.attachment_id
            );
            return None;
        }
    };
    let last_active_key = match workspace_last_active_metadata_key(&session.kind) {
        Some(key) => key,
        None => {
            log::warn!(
                "missing last-active metadata key for workspace {} session {} kind {}",
                workspace,
                session.attachment_id,
                session.kind
            );
            return None;
        }
    };
    let serialized = match serde_json::to_string(session) {
        Ok(serialized) => serialized,
        Err(error) => {
            log::warn!(
                "failed to serialize workspace session metadata for workspace {} session {}: {}",
                workspace,
                session.attachment_id,
                error
            );
            return None;
        }
    };

    Some(vec![
        WorkspaceMetadataEntry {
            key: session_key,
            value: Some(serialized),
        },
        WorkspaceMetadataEntry {
            key: last_active_key.to_string(),
            value: Some(workspaces::current_rfc3339_timestamp()),
        },
    ])
}

fn should_drop_pending_session(
    pending: &WorkspaceSession,
    metadata: Option<&WorkspaceSession>,
) -> bool {
    match metadata {
        Some(metadata_session) if metadata_session == pending => true,
        Some(_) => {
            pending.kind == "terminal" && pending.working.is_none() && pending.unread.is_none()
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

            let entry_keys = entries
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if let Err(error) = update_result {
                log::warn!(
                    "background metadata update failed for workspace {} keys=[{}]: {}",
                    workspace,
                    entry_keys,
                    error
                );
                if let Ok(mut pending) = self.metadata.lock() {
                    let state = pending.entry(workspace.clone()).or_insert_with(|| {
                        PendingWorkspaceMetadata {
                            lookup: Some(current_lookup.clone()),
                            entries: HashMap::new(),
                            worker_running: true,
                        }
                    });
                    state.lookup = Some(current_lookup);
                    requeue_workspace_metadata_entries(state, entries);
                }
            } else if let Ok(mut pending) = self.metadata.lock() {
                if let Some(state) = pending.get_mut(&workspace) {
                    state.lookup = Some(current_lookup);
                }
            }
        }
    }

    pub(crate) fn enqueue_workspace_session_upsert(
        &self,
        workspace: &str,
        lookup: Option<WorkspaceLookup>,
        session: WorkspaceSession,
    ) {
        let entries = match workspace_session_metadata_entries(workspace, &session) {
            Some(entries) => entries,
            None => return,
        };
        self.upsert_workspace_session(workspace, session);
        self.enqueue(workspace, lookup, entries);
    }

    pub(crate) fn enqueue_workspace_session_remove(
        &self,
        workspace: &str,
        lookup: Option<WorkspaceLookup>,
        kind: &str,
        attachment_id: &str,
    ) {
        let Some(key) = workspace_session_metadata_key(kind, attachment_id) else {
            log::warn!(
                "unsupported workspace session remove kind {} for workspace {} session {}",
                kind,
                workspace,
                attachment_id
            );
            return;
        };
        self.remove_workspace_session(workspace, kind, attachment_id);
        self.enqueue(
            workspace,
            lookup,
            vec![WorkspaceMetadataEntry { key, value: None }],
        );
    }

    pub(crate) fn enqueue_workspace_lifecycle(
        &self,
        workspace: &str,
        lookup: Option<WorkspaceLookup>,
        phase: &str,
        detail: Option<&str>,
        last_error: Option<&str>,
    ) {
        self.enqueue(
            workspace,
            lookup,
            workspaces::workspace_lifecycle_metadata_entries(phase, detail, last_error),
        );
    }

    pub(crate) fn enqueue_template_operation(
        &self,
        workspace: &str,
        lookup: Option<WorkspaceLookup>,
        kind: &str,
        phase: &str,
        detail: Option<&str>,
        last_error: Option<&str>,
        snapshot_name: Option<&str>,
    ) {
        self.enqueue(
            workspace,
            lookup,
            workspaces::template_operation_metadata_entries(
                kind,
                phase,
                detail,
                last_error,
                snapshot_name,
            ),
        );
    }

    pub(crate) fn upsert_workspace_session(&self, workspace: &str, session: WorkspaceSession) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        sessions.entry(workspace.to_string()).or_default().insert(
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
        let workspace = self.apply_workspace_session_state(&workspace_name, workspace);
        let workspace = self.apply_active_workspace_state(&workspace_name, workspace);
        workspaces::clear_invalid_workspace_active_session(workspace)
    }

    fn apply_workspace_session_state(
        &self,
        workspace_name: &str,
        workspace: Workspace,
    ) -> Workspace {
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
            let Some(entries) = sessions.get_mut(workspace_name) else {
                return workspace;
            };
            entries.retain(|key, pending| match pending {
                Some(session) => !should_drop_pending_session(session, metadata_sessions.get(key)),
                None => metadata_sessions.contains_key(key),
            });
            if entries.is_empty() {
                sessions.remove(workspace_name);
                return workspace;
            }
            entries.clone()
        };

        workspaces::overlay_workspace_sessions(workspace, &overlay)
    }

    fn apply_active_workspace_state(
        &self,
        workspace_name: &str,
        workspace: Workspace,
    ) -> Workspace {
        let overlay = {
            let Ok(mut active_sessions) = self.active_sessions.lock() else {
                return workspace;
            };
            let Some(pending) = active_sessions.get(workspace_name).cloned() else {
                return workspace;
            };
            if pending == workspace.active_session().cloned() {
                active_sessions.remove(workspace_name);
                return workspace;
            }
            pending
        };

        workspaces::overlay_workspace_active_session(workspace, overlay)
    }

    pub(crate) fn set_active_workspace_session(
        &self,
        workspace: &str,
        active_session: WorkspaceActiveSession,
    ) {
        let Ok(mut active_sessions) = self.active_sessions.lock() else {
            return;
        };
        active_sessions.insert(workspace.to_string(), Some(active_session));
    }

    pub(crate) fn clear_active_workspace_session_if_matches(
        &self,
        workspace: &str,
        kind: &str,
        attachment_id: &str,
        metadata_active_session: Option<&WorkspaceActiveSession>,
    ) -> bool {
        let Ok(mut active_sessions) = self.active_sessions.lock() else {
            return metadata_active_session
                .is_some_and(|active| active.matches(kind, attachment_id));
        };
        let current_active = active_sessions
            .get(workspace)
            .cloned()
            .flatten()
            .or_else(|| metadata_active_session.cloned());
        if current_active.is_some_and(|active| active.matches(kind, attachment_id)) {
            active_sessions.insert(workspace.to_string(), None);
            true
        } else {
            false
        }
    }

    pub(crate) fn apply_workspace_states(&self, workspaces: Vec<Workspace>) -> Vec<Workspace> {
        workspaces
            .into_iter()
            .map(|workspace| self.apply_workspace_state(workspace))
            .collect()
    }

    pub(crate) fn cache_transient_template_state(&self, state: TemplateState) {
        let Ok(mut template_states) = self.template_states.lock() else {
            return;
        };
        template_states.insert(
            state.project.clone(),
            TransientTemplateState {
                state,
                cached_at: Instant::now(),
            },
        );
    }

    pub(crate) fn recent_transient_template_state(&self, project: &str) -> Option<TemplateState> {
        let Ok(mut template_states) = self.template_states.lock() else {
            return None;
        };
        let cached = template_states.get(project)?.clone();
        let expired = cached.cached_at.elapsed() > TEMPLATE_TRANSIENT_STATE_TTL
            || cached
                .state
                .operation
                .as_ref()
                .is_none_or(|operation| operation.status == TemplateOperationStatus::Running);
        if expired {
            template_states.remove(project);
            return None;
        }
        Some(cached.state)
    }

    pub(crate) fn clear_transient_template_state(&self, project: &str) {
        let Ok(mut template_states) = self.template_states.lock() else {
            return;
        };
        template_states.remove(project);
    }

    pub(crate) fn begin_template_reconcile(&self, project: &str) -> bool {
        let Ok(mut template_reconciles) = self.template_reconciles.lock() else {
            return false;
        };
        if template_reconciles.get(project).copied().unwrap_or(false) {
            return false;
        }
        template_reconciles.insert(project.to_string(), true);
        true
    }

    pub(crate) fn finish_template_reconcile(&self, project: &str) {
        let Ok(mut template_reconciles) = self.template_reconciles.lock() else {
            return;
        };
        template_reconciles.remove(project);
    }
}

async fn sleep_for(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}

fn requeue_workspace_metadata_entries(
    state: &mut PendingWorkspaceMetadata,
    entries: Vec<WorkspaceMetadataEntry>,
) {
    for entry in entries {
        state.entries.entry(entry.key).or_insert(entry.value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_session(kind: &str, attachment_id: &str) -> WorkspaceSession {
        WorkspaceSession {
            kind: kind.to_string(),
            name: "sample".to_string(),
            attachment_id: attachment_id.to_string(),
            path: None,
            url: None,
            logical_url: None,
            resolved_url: None,
            title: None,
            favicon_url: None,
            can_go_back: None,
            can_go_forward: None,
            working: None,
            unread: None,
        }
    }

    #[test]
    fn workspace_session_metadata_entries_use_kind_specific_keys() {
        let browser_entries =
            workspace_session_metadata_entries("ws", &sample_session("browser", "browser-1"))
                .expect("browser entries should serialize");
        assert_eq!(browser_entries[0].key, "browser-session-browser-1");
        assert_eq!(browser_entries[1].key, "browser-last-active");

        let file_entries =
            workspace_session_metadata_entries("ws", &sample_session("file", "file-1"))
                .expect("file entries should serialize");
        assert_eq!(file_entries[0].key, "file-session-file-1");
        assert_eq!(file_entries[1].key, "file-last-active");

        let terminal_entries =
            workspace_session_metadata_entries("ws", &sample_session("terminal", "terminal-1"))
                .expect("terminal entries should serialize");
        assert_eq!(terminal_entries[0].key, "terminal-session-terminal-1");
        assert_eq!(terminal_entries[1].key, "terminal-last-active");
    }

    #[test]
    fn requeue_workspace_metadata_entries_preserves_newer_values() {
        let mut pending = PendingWorkspaceMetadata {
            lookup: None,
            entries: HashMap::from([
                (
                    "terminal-session-terminal-1".to_string(),
                    Some("new".to_string()),
                ),
                (
                    "terminal-last-active".to_string(),
                    Some("fresh".to_string()),
                ),
            ]),
            worker_running: true,
        };

        requeue_workspace_metadata_entries(
            &mut pending,
            vec![
                WorkspaceMetadataEntry {
                    key: "terminal-session-terminal-1".to_string(),
                    value: Some("old".to_string()),
                },
                WorkspaceMetadataEntry {
                    key: "active-session-kind".to_string(),
                    value: Some("terminal".to_string()),
                },
            ],
        );

        assert_eq!(
            pending.entries.get("terminal-session-terminal-1"),
            Some(&Some("new".to_string()))
        );
        assert_eq!(
            pending.entries.get("active-session-kind"),
            Some(&Some("terminal".to_string()))
        );
    }

    #[test]
    fn should_drop_pending_terminal_without_status_when_metadata_exists() {
        let pending = WorkspaceSession {
            kind: "terminal".to_string(),
            name: "codex".to_string(),
            attachment_id: "terminal-1".to_string(),
            path: None,
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
        let metadata = WorkspaceSession {
            working: Some(true),
            unread: Some(false),
            ..pending.clone()
        };

        assert!(should_drop_pending_session(&pending, Some(&metadata)));
    }
}
