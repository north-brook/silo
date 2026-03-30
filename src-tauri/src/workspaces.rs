use crate::agent_sessions::{self, WorkspaceSessionSnapshot};
use crate::bootstrap;
use crate::config::{ConfigStore, ProjectConfig, SiloConfig};
use crate::gcp;
use crate::river_names::DEFAULT_RIVER_NAMES;
use crate::state::{
    WorkspaceMetadataEntry, BROWSER_LAST_ACTIVE_METADATA_KEY, FILE_LAST_ACTIVE_METADATA_KEY,
    TERMINAL_LAST_ACTIVE_METADATA_KEY, TERMINAL_LAST_WORKING_METADATA_KEY,
    TERMINAL_UNREAD_METADATA_KEY, TERMINAL_WORKING_METADATA_KEY,
};
use crate::terminal;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tauri::State;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

pub(crate) const WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY: &str = "workspace-lifecycle-phase";
pub(crate) const WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY: &str = "workspace-lifecycle-detail";
pub(crate) const WORKSPACE_LIFECYCLE_ERROR_METADATA_KEY: &str = "workspace-lifecycle-error";
pub(crate) const WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY: &str =
    "workspace-lifecycle-updated-at";
pub(crate) const WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY: &str = "workspace-agent-heartbeat-at";
pub(crate) const WORKSPACE_AGENT_FINGERPRINT_METADATA_KEY: &str = "workspace-agent-fingerprint";
pub(crate) const TEMPLATE_OPERATION_KIND_METADATA_KEY: &str = "template-operation-kind";
pub(crate) const TEMPLATE_OPERATION_PHASE_METADATA_KEY: &str = "template-operation-phase";
pub(crate) const TEMPLATE_OPERATION_DETAIL_METADATA_KEY: &str = "template-operation-detail";
pub(crate) const TEMPLATE_OPERATION_ERROR_METADATA_KEY: &str = "template-operation-error";
pub(crate) const TEMPLATE_OPERATION_UPDATED_AT_METADATA_KEY: &str = "template-operation-updated-at";
pub(crate) const TEMPLATE_OPERATION_SNAPSHOT_METADATA_KEY: &str = "template-operation-snapshot";
pub(crate) const TEMPLATE_OPERATION_KIND_LABEL_KEY: &str = "template-operation-kind";
pub(crate) const TEMPLATE_OPERATION_PHASE_LABEL_KEY: &str = "template-operation-phase";
pub(crate) const TEMPLATE_OPERATION_ID_LABEL_KEY: &str = "template-operation-id";
const STARTUP_FAILURE_RETRY_COOLDOWN: Duration = Duration::from_secs(15);
const METADATA_DELIMITER_CANDIDATES: &[&str] = &["|", ";", "#", "@@", "SILO_METADATA_DELIM"];
const MAX_WORKSPACE_METADATA_VALUE_LEN: usize = 512;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Workspace {
    Branch(BranchWorkspace),
    Template(TemplateWorkspace),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BranchWorkspace {
    #[serde(flatten)]
    base: WorkspaceBase,
    branch: String,
    target_branch: String,
    unread: bool,
    working: Option<bool>,
    terminals: Vec<WorkspaceSession>,
    browsers: Vec<WorkspaceSession>,
    files: Vec<WorkspaceSession>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TemplateWorkspace {
    #[serde(flatten)]
    base: WorkspaceBase,
    unread: bool,
    working: Option<bool>,
    terminals: Vec<WorkspaceSession>,
    browsers: Vec<WorkspaceSession>,
    files: Vec<WorkspaceSession>,
    template: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    template_operation: Option<TemplateOperationState>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SnapshotTemplate {
    name: String,
    project: String,
    created_at: String,
    status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLifecycle {
    phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TemplateOperationState {
    pub(crate) kind: String,
    pub(crate) phase: String,
    pub(crate) detail: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) updated_at: Option<String>,
    pub(crate) snapshot_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct WorkspaceBase {
    name: String,
    project: Option<String>,
    last_active: Option<String>,
    last_working: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_session: Option<WorkspaceActiveSession>,
    created_at: String,
    status: String,
    zone: String,
    lifecycle: WorkspaceLifecycle,
    #[serde(skip_serializing)]
    agent_heartbeat_at: Option<String>,
    #[serde(skip_serializing)]
    agent_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceActiveSession {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) attachment_id: String,
}

impl WorkspaceActiveSession {
    pub(crate) fn new(kind: impl Into<String>, attachment_id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            attachment_id: attachment_id.into(),
        }
    }

    pub(crate) fn matches(&self, kind: &str, attachment_id: &str) -> bool {
        self.kind == kind && self.attachment_id == attachment_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSession {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) attachment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) logical_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) favicon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_go_back: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) can_go_forward: Option<bool>,
    pub(crate) working: Option<bool>,
    pub(crate) unread: Option<bool>,
}

fn workspace_session_key(kind: &str, attachment_id: &str) -> String {
    format!("{kind}:{attachment_id}")
}

impl Workspace {
    fn branch(
        base: WorkspaceBase,
        branch: String,
        target_branch: String,
        unread: bool,
        working: Option<bool>,
        terminals: Vec<WorkspaceSession>,
        browsers: Vec<WorkspaceSession>,
        files: Vec<WorkspaceSession>,
    ) -> Self {
        Self::Branch(BranchWorkspace {
            base,
            branch,
            target_branch,
            unread,
            working,
            terminals,
            browsers,
            files,
        })
    }

    fn base(&self) -> &WorkspaceBase {
        match self {
            Self::Branch(workspace) => &workspace.base,
            Self::Template(workspace) => &workspace.base,
        }
    }

    pub(crate) fn is_template(&self) -> bool {
        matches!(self, Self::Template(_))
    }

    fn last_active(&self) -> Option<&str> {
        self.base().last_active.as_deref()
    }

    pub(crate) fn active_session(&self) -> Option<&WorkspaceActiveSession> {
        self.base().active_session.as_ref()
    }

    pub(crate) fn name(&self) -> &str {
        &self.base().name
    }

    pub(crate) fn zone(&self) -> &str {
        &self.base().zone
    }

    pub(crate) fn status(&self) -> &str {
        &self.base().status
    }

    pub(crate) fn project(&self) -> Option<&str> {
        self.base().project.as_deref()
    }

    pub(crate) fn lifecycle(&self) -> &WorkspaceLifecycle {
        &self.base().lifecycle
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.lifecycle().is_ready()
    }

    pub(crate) fn should_reconcile_startup(&self) -> bool {
        self.lifecycle().should_reconcile(self.status())
    }

    pub(crate) fn agent_heartbeat_at(&self) -> Option<&str> {
        self.base().agent_heartbeat_at.as_deref()
    }

    pub(crate) fn agent_fingerprint(&self) -> Option<&str> {
        self.base().agent_fingerprint.as_deref()
    }

    pub(crate) fn branch_name(&self) -> Option<&str> {
        match self {
            Self::Branch(workspace) => Some(workspace.branch.as_str()),
            Self::Template(_) => None,
        }
    }

    pub(crate) fn target_branch(&self) -> Option<&str> {
        match self {
            Self::Branch(workspace) => Some(workspace.target_branch.as_str()),
            Self::Template(_) => None,
        }
    }

    pub(crate) fn terminals(&self) -> &[WorkspaceSession] {
        match self {
            Self::Branch(workspace) => &workspace.terminals,
            Self::Template(workspace) => &workspace.terminals,
        }
    }

    pub(crate) fn browsers(&self) -> &[WorkspaceSession] {
        match self {
            Self::Branch(workspace) => &workspace.browsers,
            Self::Template(workspace) => &workspace.browsers,
        }
    }

    pub(crate) fn files(&self) -> &[WorkspaceSession] {
        match self {
            Self::Branch(workspace) => &workspace.files,
            Self::Template(workspace) => &workspace.files,
        }
    }

    pub(crate) fn sessions(&self) -> Vec<WorkspaceSession> {
        let mut sessions = self.terminals().to_vec();
        sessions.extend_from_slice(self.browsers());
        sessions.extend_from_slice(self.files());
        terminal::sort_workspace_sessions_oldest_to_newest(&mut sessions);
        sessions
    }

    pub(crate) fn has_session(&self, kind: &str, attachment_id: &str) -> bool {
        self.sessions()
            .into_iter()
            .any(|session| session.kind == kind && session.attachment_id == attachment_id)
    }

    pub(crate) fn template_operation(&self) -> Option<&TemplateOperationState> {
        match self {
            Self::Template(workspace) => workspace.template_operation.as_ref(),
            Self::Branch(_) => None,
        }
    }
}

impl TemplateWorkspace {
    pub(crate) fn template_operation(&self) -> Option<&TemplateOperationState> {
        self.template_operation.as_ref()
    }
}

impl WorkspaceLifecycle {
    pub(crate) fn new(
        phase: impl Into<String>,
        detail: Option<String>,
        last_error: Option<String>,
        updated_at: Option<String>,
    ) -> Self {
        Self {
            phase: phase.into(),
            detail,
            last_error,
            updated_at,
        }
    }

    pub(crate) fn phase(&self) -> &str {
        &self.phase
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.phase == "ready"
    }

    pub(crate) fn with_updated_at(mut self, updated_at: Option<String>) -> Self {
        self.updated_at = updated_at;
        self
    }

    pub(crate) fn should_reconcile(&self, status: &str) -> bool {
        if status != "RUNNING" || self.is_ready() {
            return false;
        }
        if self.phase == "updating_workspace_agent" {
            return false;
        }
        if self.phase != "failed" {
            return true;
        }

        let Some(updated_at) = self.updated_at.as_deref() else {
            return true;
        };
        let Ok(updated_at) = OffsetDateTime::parse(updated_at, &Rfc3339) else {
            return true;
        };

        OffsetDateTime::now_utc() - updated_at >= STARTUP_FAILURE_RETRY_COOLDOWN
    }
}

pub(crate) fn overlay_workspace_sessions(
    workspace: Workspace,
    overlay: &HashMap<String, Option<WorkspaceSession>>,
) -> Workspace {
    if overlay.is_empty() {
        return workspace;
    }

    let mut sessions = workspace
        .sessions()
        .into_iter()
        .map(|session| {
            (
                workspace_session_key(&session.kind, &session.attachment_id),
                session,
            )
        })
        .collect::<HashMap<_, _>>();
    for (key, value) in overlay {
        match value {
            Some(session) => {
                sessions.insert(key.clone(), session.clone());
            }
            None => {
                sessions.remove(key);
            }
        }
    }

    let mut terminals = Vec::new();
    let mut browsers = Vec::new();
    let mut files = Vec::new();
    for session in sessions.into_values() {
        if session.kind == "browser" {
            browsers.push(session);
        } else if session.kind == "file" {
            files.push(session);
        } else {
            terminals.push(session);
        }
    }
    terminal::sort_workspace_sessions_oldest_to_newest(&mut terminals);
    terminal::sort_workspace_sessions_oldest_to_newest(&mut browsers);
    terminal::sort_workspace_sessions_oldest_to_newest(&mut files);
    let assistant_present = terminals
        .iter()
        .any(|session| session.working.is_some() || session.unread.is_some());
    let working = assistant_present.then_some(
        terminals
            .iter()
            .any(|session| session.working == Some(true)),
    );
    let unread = terminals.iter().any(|session| session.unread == Some(true));

    match workspace {
        Workspace::Branch(mut workspace) => {
            workspace.unread = unread;
            workspace.working = working;
            workspace.terminals = terminals;
            workspace.browsers = browsers;
            workspace.files = files;
            Workspace::Branch(workspace)
        }
        Workspace::Template(mut workspace) => {
            workspace.unread = unread;
            workspace.working = working;
            workspace.terminals = terminals;
            workspace.browsers = browsers;
            workspace.files = files;
            Workspace::Template(workspace)
        }
    }
}

pub(crate) fn replace_workspace_terminals(
    workspace: Workspace,
    terminals: Vec<WorkspaceSession>,
) -> Workspace {
    let mut overlay = HashMap::new();
    for session in workspace.terminals() {
        overlay.insert(
            workspace_session_key(&session.kind, &session.attachment_id),
            None,
        );
    }
    for session in terminals {
        overlay.insert(
            workspace_session_key(&session.kind, &session.attachment_id),
            Some(session),
        );
    }

    overlay_workspace_sessions(workspace, &overlay)
}

pub(crate) fn overlay_workspace_runtime_snapshot(
    workspace: Workspace,
    snapshot: WorkspaceSessionSnapshot,
) -> Workspace {
    match workspace {
        Workspace::Branch(mut workspace) => {
            workspace.base.last_active = snapshot.last_active.or(workspace.base.last_active);
            workspace.base.last_working = snapshot.last_working.or(workspace.base.last_working);
            workspace.base.active_session = snapshot.active_session;
            workspace.unread = snapshot.unread;
            workspace.working = Some(snapshot.working);
            workspace.terminals = snapshot.terminals;
            workspace.browsers = snapshot.browsers;
            workspace.files = snapshot.files;
            Workspace::Branch(workspace)
        }
        Workspace::Template(mut workspace) => {
            workspace.base.last_active = snapshot.last_active.or(workspace.base.last_active);
            workspace.base.last_working = snapshot.last_working.or(workspace.base.last_working);
            workspace.base.active_session = snapshot.active_session;
            workspace.unread = snapshot.unread;
            workspace.working = Some(snapshot.working);
            workspace.terminals = snapshot.terminals;
            workspace.browsers = snapshot.browsers;
            workspace.files = snapshot.files;
            Workspace::Template(workspace)
        }
    }
}

pub(crate) fn overlay_workspace_active_session(
    workspace: Workspace,
    active_session: Option<WorkspaceActiveSession>,
) -> Workspace {
    match workspace {
        Workspace::Branch(mut workspace) => {
            workspace.base.active_session = active_session;
            Workspace::Branch(workspace)
        }
        Workspace::Template(mut workspace) => {
            workspace.base.active_session = active_session;
            Workspace::Template(workspace)
        }
    }
}

pub(crate) fn overlay_workspace_lifecycle(
    workspace: Workspace,
    lifecycle: WorkspaceLifecycle,
) -> Workspace {
    match workspace {
        Workspace::Branch(mut workspace) => {
            workspace.base.lifecycle = lifecycle;
            Workspace::Branch(workspace)
        }
        Workspace::Template(mut workspace) => {
            workspace.base.lifecycle = lifecycle;
            Workspace::Template(workspace)
        }
    }
}

pub(crate) fn overlay_workspace_template_operation(
    workspace: Workspace,
    template_operation: Option<TemplateOperationState>,
) -> Workspace {
    match workspace {
        Workspace::Branch(workspace) => Workspace::Branch(workspace),
        Workspace::Template(mut workspace) => {
            workspace.template_operation = template_operation;
            Workspace::Template(workspace)
        }
    }
}

pub(crate) fn clear_invalid_workspace_active_session(workspace: Workspace) -> Workspace {
    let invalid = workspace
        .active_session()
        .is_some_and(|active| !workspace.has_session(&active.kind, &active.attachment_id));
    if invalid {
        overlay_workspace_active_session(workspace, None)
    } else {
        workspace
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedGcloudConfig {
    pub(crate) account: String,
    pub(crate) service_account: String,
    pub(crate) project: String,
    pub(crate) region: String,
    pub(crate) zone: String,
    pub(crate) machine_type: String,
    pub(crate) disk_size_gb: u32,
    pub(crate) disk_type: String,
    pub(crate) image_family: String,
    pub(crate) image_project: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceLookup {
    pub(crate) workspace: Workspace,
    pub(crate) account: String,
    pub(crate) gcloud_project: String,
}

pub(crate) async fn fetch_workspace_runtime_workspace(
    lookup: &WorkspaceLookup,
) -> Option<Workspace> {
    if !lookup.workspace.is_ready() {
        return None;
    }

    match agent_sessions::fetch_session_snapshot(lookup).await {
        Ok(snapshot) => Some(overlay_workspace_runtime_snapshot(
            lookup.workspace.clone(),
            snapshot,
        )),
        Err(error) => {
            log::warn!(
                "failed to hydrate workspace runtime sessions workspace={}: {}",
                lookup.workspace.name(),
                error
            );
            None
        }
    }
}

pub(crate) async fn hydrate_workspace_lookup(lookup: WorkspaceLookup) -> WorkspaceLookup {
    let WorkspaceLookup {
        workspace,
        account,
        gcloud_project,
    } = lookup;

    let lookup = WorkspaceLookup {
        workspace: workspace.clone(),
        account: account.clone(),
        gcloud_project: gcloud_project.clone(),
    };
    let workspace = fetch_workspace_runtime_workspace(&lookup)
        .await
        .unwrap_or(workspace);
    let workspace = apply_current_workspace_state(workspace);

    WorkspaceLookup {
        workspace,
        account,
        gcloud_project,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceBootSource {
    ImageFamily,
    Snapshot(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) name: String,
    pub(crate) created_at: String,
    pub(crate) status: String,
    pub(crate) project: Option<String>,
    pub(crate) template: bool,
    pub(crate) template_operation_kind: Option<String>,
    pub(crate) template_operation_phase: Option<String>,
    pub(crate) template_operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceState {
    status: String,
    boot_disk: String,
}

const TEMPLATE_SNAPSHOT_STATUS_READY: &str = "READY";
const INSTANCE_STATUS_TERMINATED: &str = "TERMINATED";
const TEMPLATE_STOP_POLL_ATTEMPTS: usize = 90;
const TEMPLATE_STOP_POLL_INTERVAL: Duration = Duration::from_secs(2);
const TEMPLATE_SNAPSHOT_POLL_ATTEMPTS: usize = 60;
const TEMPLATE_SNAPSHOT_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[tauri::command]
pub async fn workspaces_list_workspaces(
    state: State<'_, crate::state::WorkspaceMetadataManager>,
) -> Result<Vec<Workspace>, String> {
    let workspaces = list_all_workspaces().await?;
    let workspaces = state.apply_workspace_states(workspaces);
    for workspace in &workspaces {
        bootstrap::start_workspace_startup_reconcile_if_needed(workspace.clone());
        bootstrap::start_workspace_agent_update_reconcile_if_needed(workspace.clone());
    }

    Ok(workspaces)
}

pub(crate) async fn list_all_workspaces() -> Result<Vec<Workspace>, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let candidates = candidate_gcloud_configs(&config);

    if candidates.is_empty() {
        return Err("gcloud account and project must be configured".to_string());
    }

    let mut workspaces = Vec::new();
    for gcloud in candidates {
        workspaces.extend(list_workspaces_in_project(&gcloud.account, &gcloud.project).await?);
    }
    workspaces.sort_by(compare_workspaces_by_last_active_desc);

    Ok(workspaces)
}

#[tauri::command]
pub async fn workspaces_create_workspace(project: String) -> Result<Workspace, String> {
    log::info!("creating workspace for project {project}");
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project_config = config
        .projects
        .get(&project)
        .ok_or_else(|| format!("project not found: {project}"))?;
    let gcloud = resolve_project_gcloud_config(&config, &project)?;
    let candidates = candidate_gcloud_configs(&config);
    let (workspace_name, branch_name) =
        reserve_branch_workspace_identity(&project, &gcloud.account, &gcloud.project, &candidates)
            .await?;
    let boot_source = latest_template_snapshot_name(&gcloud.account, &gcloud.project, &project)
        .await?
        .map(WorkspaceBootSource::Snapshot)
        .unwrap_or(WorkspaceBootSource::ImageFamily);
    let request = create_workspace_request_body(
        &workspace_name,
        &project,
        &branch_name,
        &project_config.target_branch,
        &boot_source,
        &gcloud,
    )
    .await?;
    gcp::create_instance(&gcloud.project, &gcloud.zone, request).await?;

    bootstrap::cache_workspace_bootstrap(
        &workspace_name,
        &config,
        &project,
        project_config,
        &project_config.target_branch,
        Some(&branch_name),
    )?;
    log::info!("workspace {workspace_name} creation started for project {project}");
    bootstrap::start_workspace_startup_reconcile(workspace_name.clone());
    match describe_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project).await {
        Ok(workspace) => Ok(workspace),
        Err(error) => {
            log::warn!(
                "workspace {} creation started but instance is not yet visible: {}",
                workspace_name,
                error
            );
            Ok(pending_workspace(
                &workspace_name,
                &project,
                &branch_name,
                &project_config.target_branch,
                &gcloud.zone,
            ))
        }
    }
}

#[tauri::command]
pub async fn workspaces_start_workspace(workspace: String) -> Result<(), String> {
    log::info!("starting workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    gcp::post_instance_action(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        &workspace,
        "start",
        "failed to start workspace",
    )
    .await?;

    bootstrap::start_workspace_startup_reconcile(workspace.clone());

    log::info!("workspace {} started", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_resume_workspace(workspace: String) -> Result<(), String> {
    log::info!("resuming workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    gcp::post_instance_action(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        &workspace,
        "resume",
        "failed to resume workspace",
    )
    .await?;

    bootstrap::start_workspace_startup_reconcile(workspace.clone());

    log::info!("workspace {} resume initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_stop_workspace(workspace: String) -> Result<(), String> {
    log::info!("stopping workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    gcp::post_instance_action(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        &workspace,
        "stop",
        "failed to stop workspace",
    )
    .await?;

    log::info!("workspace {} stop initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_suspend_workspace(workspace: String) -> Result<(), String> {
    log::info!("suspending workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    gcp::post_instance_action(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        &workspace,
        "suspend",
        "failed to suspend workspace",
    )
    .await?;

    log::info!("workspace {} suspend initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_get_workspace(
    state: State<'_, crate::state::WorkspaceMetadataManager>,
    workspace: String,
) -> Result<Workspace, String> {
    log::trace!("getting workspace {workspace}");
    let lookup = find_workspace_raw(&workspace).await?;
    let runtime_workspace = fetch_workspace_runtime_workspace(&lookup).await;
    state.reconcile_workspace_observation(&lookup.workspace, runtime_workspace.as_ref());
    let workspace =
        state.apply_workspace_state(runtime_workspace.unwrap_or_else(|| lookup.workspace.clone()));
    bootstrap::start_workspace_startup_reconcile_if_needed(workspace.clone());
    bootstrap::start_workspace_agent_update_reconcile_if_needed(workspace.clone());
    Ok(workspace)
}

#[tauri::command]
pub async fn workspaces_set_active_session(
    state: State<'_, crate::state::WorkspaceMetadataManager>,
    workspace: String,
    kind: String,
    attachment_id: String,
) -> Result<(), String> {
    let kind = kind.trim().to_string();
    let attachment_id = attachment_id.trim().to_string();
    if kind.is_empty() {
        return Err("active session kind must not be empty".to_string());
    }
    if attachment_id.is_empty() {
        return Err("active session attachment_id must not be empty".to_string());
    }

    let active_session = WorkspaceActiveSession::new(kind.clone(), attachment_id);
    state.set_active_workspace_session(&workspace, active_session.clone());
    let lookup = find_workspace_raw(&workspace).await?;
    let runtime_workspace = fetch_workspace_runtime_workspace(&lookup).await;
    state.reconcile_workspace_observation(&lookup.workspace, runtime_workspace.as_ref());
    let workspace_with_state =
        state.apply_workspace_state(runtime_workspace.unwrap_or_else(|| lookup.workspace.clone()));
    if !workspace_with_state.has_session(&kind, &active_session.attachment_id) {
        log::info!(
            "ignoring active session update for missing workspace session workspace={} kind={} attachment_id={}",
            workspace,
            kind,
            active_session.attachment_id
        );
        state.clear_active_workspace_session_if_matches(
            &workspace,
            &kind,
            &active_session.attachment_id,
            None,
        );
        return Ok(());
    }
    if lookup.workspace.is_ready() {
        agent_sessions::set_active_session(&lookup, Some(&active_session)).await?;
    }

    Ok(())
}

#[tauri::command]
pub async fn workspaces_submit_prompt(
    terminal_state: State<'_, crate::terminal::TerminalManager>,
    workspace_state: State<'_, crate::state::WorkspaceMetadataManager>,
    workspace: String,
    prompt: String,
    model: String,
) -> Result<terminal::TerminalCreateResult, String> {
    log::info!("submitting {model} prompt for workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    if !lookup.workspace.is_ready() {
        bootstrap::start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());
        return Err(workspace_not_ready_error(&lookup.workspace));
    }

    let prompt = trim_prompt_input(&prompt)?;
    let provider = assistant_provider_for_model(&model)?;
    let attachment_id = terminal::start_assistant_session(
        terminal_state.inner(),
        workspace_state.inner(),
        &workspace,
        provider,
        Some(&prompt),
    )
    .await?;

    Ok(terminal::TerminalCreateResult { attachment_id })
}

#[tauri::command]
pub async fn workspaces_delete_workspace(workspace: String) -> Result<(), String> {
    log::info!("deleting workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    gcp::delete_instance(&lookup.gcloud_project, lookup.workspace.zone(), &workspace).await?;

    log::info!("workspace {} delete initiated", lookup.workspace.name());
    Ok(())
}

pub(crate) async fn set_workspace_metadata(
    workspace: &str,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let lookup = find_workspace(workspace).await?;
    update_workspace_metadata_in_lookup(lookup, key, value).await
}

fn trim_prompt_input(prompt: &str) -> Result<String, String> {
    if prompt.trim().is_empty() {
        return Err("prompt must not be empty".to_string());
    }

    Ok(prompt.to_string())
}

fn assistant_provider_for_model(model: &str) -> Result<terminal::AssistantProvider, String> {
    terminal::AssistantProvider::parse(model)
}

pub(crate) async fn find_workspace_raw(name: &str) -> Result<WorkspaceLookup, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let candidates = candidate_gcloud_configs(&config);

    if candidates.is_empty() {
        return Err("gcloud account and project must be configured".to_string());
    }

    let mut matches = Vec::new();
    for gcloud in candidates {
        if let Some(workspace) =
            find_workspace_in_project(name, &gcloud.account, &gcloud.project).await?
        {
            matches.push(WorkspaceLookup {
                workspace,
                account: gcloud.account.clone(),
                gcloud_project: gcloud.project.clone(),
            });
        }
    }

    match matches.len() {
        0 => Err(format!("workspace not found: {name}")),
        1 => Ok(matches.remove(0)),
        _ => Err(format!(
            "workspace {name} is ambiguous across multiple gcloud projects"
        )),
    }
}

pub(crate) async fn find_workspace(name: &str) -> Result<WorkspaceLookup, String> {
    let mut lookup = find_workspace_raw(name).await?;
    lookup.workspace = apply_current_workspace_state(lookup.workspace);
    Ok(lookup)
}

fn apply_current_workspace_state(workspace: Workspace) -> Workspace {
    if let Some(manager) = crate::state::current_workspace_metadata_manager() {
        manager.apply_workspace_state(workspace)
    } else {
        workspace
    }
}

async fn update_workspace_metadata_in_lookup(
    lookup: WorkspaceLookup,
    key: &str,
    value: &str,
) -> Result<(), String> {
    apply_workspace_metadata_entries_in_lookup(
        lookup,
        &[WorkspaceMetadataEntry {
            key: key.to_string(),
            value: Some(value.to_string()),
        }],
    )
    .await
}

pub(crate) fn template_operation_metadata_entries_with_updated_at(
    kind: &str,
    phase: &str,
    detail: Option<&str>,
    last_error: Option<&str>,
    snapshot_name: Option<&str>,
    updated_at: &str,
) -> Vec<WorkspaceMetadataEntry> {
    vec![
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_KIND_METADATA_KEY.to_string(),
            value: Some(kind.to_string()),
        },
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_PHASE_METADATA_KEY.to_string(),
            value: Some(phase.to_string()),
        },
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_DETAIL_METADATA_KEY.to_string(),
            value: detail.map(str::to_string),
        },
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_ERROR_METADATA_KEY.to_string(),
            value: last_error.map(str::to_string),
        },
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_UPDATED_AT_METADATA_KEY.to_string(),
            value: Some(updated_at.to_string()),
        },
        WorkspaceMetadataEntry {
            key: TEMPLATE_OPERATION_SNAPSHOT_METADATA_KEY.to_string(),
            value: snapshot_name.map(str::to_string),
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GcloudResourceErrorKind {
    NotFound,
    MetadataFingerprintConflict,
    Other,
}

pub(crate) fn classify_gcloud_resource_error(error: &str) -> GcloudResourceErrorKind {
    let lower = error.to_ascii_lowercase();
    if lower.contains("supplied fingerprint does not match current metadata fingerprint")
        || (lower.contains("metadata fingerprint") && lower.contains("does not match"))
        || lower.contains("status 412")
        || lower.contains("conditionnotmet")
    {
        GcloudResourceErrorKind::MetadataFingerprintConflict
    } else if lower.contains("was not found")
        || (lower.contains("not found")
            && (lower.contains("resource")
                || lower.contains("httperror 404")
                || lower.contains("http error 404")
                || lower.contains("status 404")))
    {
        GcloudResourceErrorKind::NotFound
    } else {
        GcloudResourceErrorKind::Other
    }
}

pub(crate) fn gcloud_resource_was_not_found(error: &str) -> bool {
    matches!(
        classify_gcloud_resource_error(error),
        GcloudResourceErrorKind::NotFound
    )
}

pub(crate) async fn apply_workspace_metadata_entries_in_lookup(
    lookup: WorkspaceLookup,
    entries: &[WorkspaceMetadataEntry],
) -> Result<(), String> {
    if entries.is_empty() {
        return Err("workspace metadata update did not include any values".to_string());
    }

    let updates = entries
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_deref()))
        .collect::<Vec<_>>();
    gcp::set_instance_metadata(
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        lookup.workspace.name(),
        &updates,
    )
    .await?;

    log::info!(
        "updated metadata keys [{}] for workspace {}",
        entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        lookup.workspace.name()
    );
    Ok(())
}

pub(crate) async fn find_workspace_in_project(
    name: &str,
    account: &str,
    project: &str,
) -> Result<Option<Workspace>, String> {
    let mut workspaces = list_workspaces_in_project(account, project)
        .await?
        .into_iter()
        .filter(|workspace| workspace.name() == name)
        .collect::<Vec<_>>();
    if workspaces.len() > 1 {
        return Err(format!(
            "workspace {name} matched multiple instances in gcloud project {project}"
        ));
    }

    Ok(workspaces.pop())
}

async fn list_workspaces_in_project(
    account: &str,
    project: &str,
) -> Result<Vec<Workspace>, String> {
    let _ = account;
    parse_workspaces_from_values(gcp::list_instances(project).await?)
}

fn pending_workspace(
    name: &str,
    project_label: &str,
    branch_label: &str,
    target_branch: &str,
    zone: &str,
) -> Workspace {
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    Workspace::branch(
        WorkspaceBase {
            name: name.to_string(),
            project: Some(sanitize_label_value(project_label)),
            last_active: None,
            last_working: None,
            active_session: None,
            created_at,
            status: "PROVISIONING".to_string(),
            zone: zone.to_string(),
            lifecycle: lifecycle_for_status("PROVISIONING", None, None, None),
            agent_heartbeat_at: None,
            agent_fingerprint: None,
        },
        branch_label.to_string(),
        target_branch.to_string(),
        false,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
}

fn pending_template_workspace(name: &str, project_label: &str, zone: &str) -> TemplateWorkspace {
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    TemplateWorkspace {
        base: WorkspaceBase {
            name: name.to_string(),
            project: Some(sanitize_label_value(project_label)),
            last_active: None,
            last_working: None,
            active_session: None,
            created_at,
            status: "PROVISIONING".to_string(),
            zone: zone.to_string(),
            lifecycle: lifecycle_for_status("PROVISIONING", None, None, None),
            agent_heartbeat_at: None,
            agent_fingerprint: None,
        },
        unread: false,
        working: None,
        terminals: Vec::new(),
        browsers: Vec::new(),
        files: Vec::new(),
        template: true,
        template_operation: None,
    }
}

pub(crate) async fn describe_workspace_in_project(
    name: &str,
    account: &str,
    project: &str,
) -> Result<Workspace, String> {
    find_workspace_in_project(name, account, project)
        .await?
        .ok_or_else(|| format!("workspace not found after creation: {name}"))
}

pub(crate) async fn create_template_workspace_for_project(
    project: &str,
    boot_source: Option<String>,
) -> Result<TemplateWorkspace, String> {
    log::info!("creating template workspace for project {project}");
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project_config = config
        .projects
        .get(project)
        .ok_or_else(|| format!("project not found: {project}"))?;

    let gcloud = resolve_project_gcloud_config(&config, project)?;
    let workspace_name = generate_template_workspace_name(project);

    if let Some(existing) =
        find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project).await?
    {
        return match existing {
            Workspace::Template(workspace) => Err(format!(
                "template workspace already exists for project {project}: {}",
                workspace.base.name
            )),
            Workspace::Branch(_) => Err(format!(
                "workspace name is already in use for project {project}: {workspace_name}"
            )),
        };
    }

    let request = create_template_workspace_request_body(
        &workspace_name,
        project,
        boot_source.as_deref(),
        &gcloud,
    )
    .await?;
    gcp::create_instance(&gcloud.project, &gcloud.zone, request).await?;

    bootstrap::cache_workspace_bootstrap(
        &workspace_name,
        &config,
        project,
        project_config,
        &project_config.target_branch,
        None,
    )?;
    log::info!(
        "template workspace {} creation started for project {}",
        workspace_name,
        project
    );
    match describe_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project).await {
        Ok(workspace) => workspace_into_template(workspace),
        Err(error) => {
            log::warn!(
                "template workspace {} creation started but instance is not yet visible: {}",
                workspace_name,
                error
            );
            Ok(pending_template_workspace(
                &workspace_name,
                project,
                &gcloud.zone,
            ))
        }
    }
}

pub(crate) fn resolve_project_gcloud_config(
    config: &SiloConfig,
    project: &str,
) -> Result<ResolvedGcloudConfig, String> {
    let project_config = config
        .projects
        .get(project)
        .ok_or_else(|| format!("project not found: {project}"))?;

    validate_required_gcloud_fields(&resolve_gcloud_config(config, project_config))
}

fn resolve_gcloud_config(config: &SiloConfig, project: &ProjectConfig) -> ResolvedGcloudConfig {
    let overrides = &project.gcloud;
    let account = if config.gcloud.service_account.trim().is_empty() {
        override_string(&config.gcloud.account, overrides.account.as_deref())
    } else {
        config.gcloud.service_account.clone()
    };

    ResolvedGcloudConfig {
        account,
        service_account: config.gcloud.service_account.clone(),
        project: override_string(&config.gcloud.project, overrides.project.as_deref()),
        region: override_string(&config.gcloud.region, overrides.region.as_deref()),
        zone: override_string(&config.gcloud.zone, overrides.zone.as_deref()),
        machine_type: override_string(
            &config.gcloud.machine_type,
            overrides.machine_type.as_deref(),
        ),
        disk_size_gb: overrides
            .disk_size_gb
            .filter(|disk_size| *disk_size > 0)
            .unwrap_or(config.gcloud.disk_size_gb),
        disk_type: override_string(&config.gcloud.disk_type, overrides.disk_type.as_deref()),
        image_family: override_string(
            &config.gcloud.image_family,
            overrides.image_family.as_deref(),
        ),
        image_project: override_string(
            &config.gcloud.image_project,
            overrides.image_project.as_deref(),
        ),
    }
}

fn override_string(default_value: &str, override_value: Option<&str>) -> String {
    override_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_value)
        .to_string()
}

fn preferred_gcloud_account(gcloud: &crate::config::GcloudConfig) -> &str {
    if gcloud.service_account.trim().is_empty() {
        &gcloud.account
    } else {
        &gcloud.service_account
    }
}

pub(crate) fn candidate_gcloud_configs(config: &SiloConfig) -> Vec<ResolvedGcloudConfig> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for project in config.projects.values() {
        let Ok(resolved) = validate_required_gcloud_fields(&resolve_gcloud_config(config, project))
        else {
            continue;
        };
        let key = (resolved.account.clone(), resolved.project.clone());
        if seen.insert(key) {
            candidates.push(resolved);
        }
    }

    let global = ResolvedGcloudConfig {
        account: preferred_gcloud_account(&config.gcloud).to_string(),
        service_account: config.gcloud.service_account.clone(),
        project: config.gcloud.project.clone(),
        region: config.gcloud.region.clone(),
        zone: config.gcloud.zone.clone(),
        machine_type: config.gcloud.machine_type.clone(),
        disk_size_gb: config.gcloud.disk_size_gb,
        disk_type: config.gcloud.disk_type.clone(),
        image_family: config.gcloud.image_family.clone(),
        image_project: config.gcloud.image_project.clone(),
    };

    if let Ok(global) = validate_required_gcloud_fields(&global) {
        if candidates.is_empty() || seen.insert((global.account.clone(), global.project.clone())) {
            candidates.push(global);
        }
    }

    candidates
}

fn validate_required_gcloud_fields(
    gcloud: &ResolvedGcloudConfig,
) -> Result<ResolvedGcloudConfig, String> {
    if gcloud.account.trim().is_empty() {
        return Err("gcloud account is not configured".to_string());
    }
    if gcloud.project.trim().is_empty() {
        return Err("gcloud project is not configured".to_string());
    }

    Ok(gcloud.clone())
}

async fn create_workspace_request_body(
    workspace_name: &str,
    project_label: &str,
    branch_label: &str,
    target_branch: &str,
    boot_source: &WorkspaceBootSource,
    gcloud: &ResolvedGcloudConfig,
) -> Result<Value, String> {
    let labels = json!({
        "project": sanitize_label_value(project_label),
    });
    let lifecycle_updated_at = current_rfc3339_timestamp();
    let source_image = match boot_source {
        WorkspaceBootSource::ImageFamily => {
            Some(gcp::get_image_from_family(&gcloud.image_project, &gcloud.image_family).await?)
        }
        WorkspaceBootSource::Snapshot(_) => None,
    };
    let source_snapshot = match boot_source {
        WorkspaceBootSource::ImageFamily => None,
        WorkspaceBootSource::Snapshot(snapshot) => Some(format!(
            "projects/{}/global/snapshots/{snapshot}",
            gcloud.project
        )),
    };

    Ok(instance_insert_body(
        workspace_name,
        &gcloud.zone,
        &gcloud.machine_type,
        gcloud.disk_size_gb,
        &gcloud.disk_type,
        labels,
        vec![
            ("branch".to_string(), branch_label.to_string()),
            ("target_branch".to_string(), target_branch.to_string()),
            (
                WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
                "provisioning".to_string(),
            ),
            (
                WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY.to_string(),
                "Provisioning virtual machine".to_string(),
            ),
            (
                WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY.to_string(),
                lifecycle_updated_at,
            ),
            ("enable-oslogin".to_string(), "TRUE".to_string()),
        ],
        source_image,
        source_snapshot,
        gcloud.service_account.trim(),
    ))
}

async fn create_template_workspace_request_body(
    workspace_name: &str,
    project_label: &str,
    source_snapshot: Option<&str>,
    gcloud: &ResolvedGcloudConfig,
) -> Result<Value, String> {
    let lifecycle_updated_at = current_rfc3339_timestamp();
    let source_image = match source_snapshot {
        Some(_) => None,
        None => {
            Some(gcp::get_image_from_family(&gcloud.image_project, &gcloud.image_family).await?)
        }
    };
    let source_snapshot = source_snapshot
        .map(|snapshot| format!("projects/{}/global/snapshots/{snapshot}", gcloud.project));
    Ok(instance_insert_body(
        workspace_name,
        &gcloud.zone,
        &gcloud.machine_type,
        gcloud.disk_size_gb,
        &gcloud.disk_type,
        json!({
            "project": sanitize_label_value(project_label),
            "template": "true",
        }),
        vec![
            (
                WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
                "provisioning".to_string(),
            ),
            (
                WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY.to_string(),
                "Provisioning virtual machine".to_string(),
            ),
            (
                WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY.to_string(),
                lifecycle_updated_at,
            ),
            ("enable-oslogin".to_string(), "TRUE".to_string()),
        ],
        source_image,
        source_snapshot,
        gcloud.service_account.trim(),
    ))
}

fn instance_insert_body(
    workspace_name: &str,
    zone: &str,
    machine_type: &str,
    disk_size_gb: u32,
    disk_type: &str,
    labels: Value,
    metadata_entries: Vec<(String, String)>,
    source_image: Option<String>,
    source_snapshot: Option<String>,
    service_account: &str,
) -> Value {
    let mut initialize_params = Map::new();
    initialize_params.insert("diskSizeGb".to_string(), json!(disk_size_gb));
    initialize_params.insert(
        "diskType".to_string(),
        Value::String(format!("zones/{zone}/diskTypes/{disk_type}")),
    );
    if let Some(source_image) = source_image {
        initialize_params.insert("sourceImage".to_string(), Value::String(source_image));
    }
    if let Some(source_snapshot) = source_snapshot {
        initialize_params.insert("sourceSnapshot".to_string(), Value::String(source_snapshot));
    }
    let metadata = metadata_entries
        .into_iter()
        .map(|(key, value)| {
            json!({
                "key": key,
                "value": value,
            })
        })
        .collect::<Vec<_>>();
    let service_accounts = if service_account.trim().is_empty() {
        Value::Array(Vec::new())
    } else {
        json!([{
            "email": service_account,
            "scopes": ["https://www.googleapis.com/auth/compute"],
        }])
    };
    json!({
        "name": workspace_name,
        "machineType": format!("zones/{zone}/machineTypes/{machine_type}"),
        "labels": labels,
        "metadata": {
            "items": metadata,
        },
        "disks": [{
            "boot": true,
            "autoDelete": true,
            "initializeParams": Value::Object(initialize_params),
        }],
        "networkInterfaces": [{
            "network": "global/networks/default",
            "accessConfigs": [{
                "name": "External NAT",
                "type": "ONE_TO_ONE_NAT",
            }],
        }],
        "serviceAccounts": service_accounts,
    })
}

fn create_workspace_args(
    workspace_name: &str,
    project_label: &str,
    branch_label: &str,
    target_branch: &str,
    boot_source: &WorkspaceBootSource,
    gcloud: &ResolvedGcloudConfig,
) -> Vec<String> {
    let labels = vec![format!("project={}", sanitize_label_value(project_label))];
    let lifecycle_updated_at = current_rfc3339_timestamp();
    let metadata_entries = vec![
        ("branch".to_string(), branch_label.to_string()),
        ("target_branch".to_string(), target_branch.to_string()),
        (
            WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
            "provisioning".to_string(),
        ),
        (
            WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY.to_string(),
            "Provisioning virtual machine".to_string(),
        ),
        (
            WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY.to_string(),
            lifecycle_updated_at,
        ),
    ];
    let metadata_pairs = metadata_entries
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let metadata = workspace_metadata_arg(&metadata_pairs)
        .expect("workspace metadata values must fit supported gcloud metadata delimiters");

    let mut args = vec![
        "compute".to_string(),
        "instances".to_string(),
        "create".to_string(),
        workspace_name.to_string(),
        format!("--zone={}", gcloud.zone),
        format!("--machine-type={}", gcloud.machine_type),
        format!("--boot-disk-size={}GB", gcloud.disk_size_gb),
        format!("--boot-disk-type={}", gcloud.disk_type),
        format!("--labels={}", labels.join(",")),
        metadata,
        "--async".to_string(),
    ];

    match boot_source {
        WorkspaceBootSource::ImageFamily => {
            args.push(format!("--image-family={}", gcloud.image_family));
            args.push(format!("--image-project={}", gcloud.image_project));
        }
        WorkspaceBootSource::Snapshot(snapshot) => {
            args.push(format!("--source-snapshot={snapshot}"));
        }
    }

    if gcloud.service_account.trim().is_empty() {
        args.push("--no-service-account".to_string());
        args.push("--no-scopes".to_string());
    } else {
        args.push(format!("--service-account={}", gcloud.service_account));
        args.push("--scopes=https://www.googleapis.com/auth/compute".to_string());
    }

    args
}

pub(crate) fn create_template_workspace_args(
    workspace_name: &str,
    project_label: &str,
    source_snapshot: Option<&str>,
    gcloud: &ResolvedGcloudConfig,
) -> Vec<String> {
    let lifecycle_updated_at = current_rfc3339_timestamp();
    let metadata_entries = vec![
        (
            WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
            "provisioning".to_string(),
        ),
        (
            WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY.to_string(),
            "Provisioning virtual machine".to_string(),
        ),
        (
            WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY.to_string(),
            lifecycle_updated_at,
        ),
    ];
    let metadata_pairs = metadata_entries
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let metadata = workspace_metadata_arg(&metadata_pairs)
        .expect("workspace metadata values must fit supported gcloud metadata delimiters");
    let mut args = vec![
        "compute".to_string(),
        "instances".to_string(),
        "create".to_string(),
        workspace_name.to_string(),
        format!("--zone={}", gcloud.zone),
        format!("--machine-type={}", gcloud.machine_type),
        format!("--boot-disk-size={}GB", gcloud.disk_size_gb),
        format!("--boot-disk-type={}", gcloud.disk_type),
        format!(
            "--labels=project={},template=true",
            sanitize_label_value(project_label)
        ),
        metadata,
        "--async".to_string(),
    ];

    if let Some(source_snapshot) = source_snapshot {
        args.push(format!("--source-snapshot={source_snapshot}"));
    } else {
        args.push(format!("--image-family={}", gcloud.image_family));
        args.push(format!("--image-project={}", gcloud.image_project));
    }

    if gcloud.service_account.trim().is_empty() {
        args.push("--no-service-account".to_string());
        args.push("--no-scopes".to_string());
    } else {
        args.push(format!("--service-account={}", gcloud.service_account));
        args.push("--scopes=https://www.googleapis.com/auth/compute".to_string());
    }

    args
}

fn create_template_snapshot_args(
    snapshot_name: &str,
    source_disk: &str,
    zone: &str,
    project_label: &str,
) -> Vec<String> {
    vec![
        "compute".to_string(),
        "snapshots".to_string(),
        "create".to_string(),
        snapshot_name.to_string(),
        format!("--source-disk={source_disk}"),
        format!("--source-disk-zone={zone}"),
        format!(
            "--labels=project={},template=true",
            sanitize_label_value(project_label)
        ),
        "--async".to_string(),
    ]
}

fn add_workspace_metadata_args(
    workspace: &Workspace,
    entries: &[(&str, &str)],
) -> Result<Vec<String>, String> {
    if entries.is_empty() {
        return Err("workspace metadata update did not include any values".to_string());
    }
    Ok(vec![
        "compute".to_string(),
        "instances".to_string(),
        "add-metadata".to_string(),
        workspace.name().to_string(),
        format!("--zone={}", workspace.zone()),
        workspace_metadata_arg(entries)?,
    ])
}

fn remove_workspace_metadata_args(workspace: &Workspace, keys: &[&str]) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "remove-metadata".to_string(),
        workspace.name().to_string(),
        format!("--zone={}", workspace.zone()),
        format!("--keys={}", keys.join(",")),
    ]
}

fn stop_workspace_args(workspace_name: &str, zone: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "stop".to_string(),
        workspace_name.to_string(),
        format!("--zone={zone}"),
        "--async".to_string(),
    ]
}

fn suspend_workspace_args(workspace_name: &str, zone: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "suspend".to_string(),
        workspace_name.to_string(),
        format!("--zone={zone}"),
        "--async".to_string(),
    ]
}

fn resume_workspace_args(workspace_name: &str, zone: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "resume".to_string(),
        workspace_name.to_string(),
        format!("--zone={zone}"),
        "--async".to_string(),
    ]
}

pub(crate) fn delete_workspace_args(workspace_name: &str, zone: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "instances".to_string(),
        "delete".to_string(),
        workspace_name.to_string(),
        format!("--zone={zone}"),
        "--quiet".to_string(),
    ]
}

pub(crate) fn delete_snapshot_args(snapshot_name: &str) -> Vec<String> {
    vec![
        "compute".to_string(),
        "snapshots".to_string(),
        "delete".to_string(),
        snapshot_name.to_string(),
        "--quiet".to_string(),
    ]
}

pub(crate) async fn ensure_template_workspace_terminated(
    account: &str,
    gcloud_project: &str,
    workspace_name: &str,
    zone: &str,
) -> Result<String, String> {
    let instance =
        describe_instance_in_project(workspace_name, account, gcloud_project, zone).await?;
    let instance = if instance.status == INSTANCE_STATUS_TERMINATED {
        instance
    } else {
        gcp::post_instance_action(
            gcloud_project,
            zone,
            workspace_name,
            "stop",
            "failed to stop template workspace",
        )
        .await?;

        wait_for_instance_terminated(account, gcloud_project, workspace_name, zone).await?
    };

    Ok(instance.boot_disk)
}

pub(crate) async fn delete_template_workspace_if_exists(
    account: &str,
    gcloud_project: &str,
    workspace_name: &str,
    _zone: &str,
) -> Result<(), String> {
    let workspace = find_workspace_in_project(workspace_name, account, gcloud_project).await?;
    let Some(workspace) = workspace else {
        return Ok(());
    };
    gcp::delete_instance(gcloud_project, workspace.zone(), workspace_name).await?;

    Ok(())
}

pub(crate) async fn create_template_snapshot_for_disk_named(
    account: &str,
    gcloud_project: &str,
    project: &str,
    snapshot_name: &str,
    source_disk: &str,
    zone: &str,
) -> Result<(), String> {
    let _ = account;
    gcp::create_snapshot(
        gcloud_project,
        json!({
            "name": snapshot_name,
            "sourceDisk": format!("projects/{gcloud_project}/zones/{zone}/disks/{source_disk}"),
            "labels": {
                "project": sanitize_label_value(project),
                "template": "true",
            },
        }),
    )
    .await?;
    Ok(())
}

pub(crate) async fn delete_old_template_snapshots(
    account: &str,
    gcloud_project: &str,
    project: &str,
    keep_snapshot_name: &str,
) -> Result<(), String> {
    let snapshots = list_template_snapshots_in_project(account, gcloud_project, project).await?;
    for snapshot in snapshots
        .into_iter()
        .filter(|snapshot| snapshot.name != keep_snapshot_name)
    {
        if gcp::delete_snapshot(gcloud_project, &snapshot.name)
            .await
            .is_ok()
        {
            log::info!(
                "deleted older template snapshot {} for project {}",
                snapshot.name,
                project
            );
        } else {
            log::warn!(
                "failed to delete older template snapshot {} for project {}",
                snapshot.name,
                project
            );
        }
    }

    Ok(())
}

pub(crate) async fn delete_template_snapshots(
    account: &str,
    gcloud_project: &str,
    project: &str,
) -> Result<(), String> {
    let snapshots = list_template_snapshots_in_project(account, gcloud_project, project).await?;
    for snapshot in snapshots {
        gcp::delete_snapshot(gcloud_project, &snapshot.name).await?;
    }

    Ok(())
}

pub(crate) async fn wait_for_instance_terminated(
    account: &str,
    gcloud_project: &str,
    workspace_name: &str,
    zone: &str,
) -> Result<InstanceState, String> {
    for attempt in 0..TEMPLATE_STOP_POLL_ATTEMPTS {
        let instance =
            describe_instance_in_project(workspace_name, account, gcloud_project, zone).await?;
        if instance.status == INSTANCE_STATUS_TERMINATED {
            return Ok(instance);
        }

        if attempt + 1 < TEMPLATE_STOP_POLL_ATTEMPTS {
            sleep_for(TEMPLATE_STOP_POLL_INTERVAL).await;
        }
    }

    Err(format!(
        "template workspace {workspace_name} did not reach {INSTANCE_STATUS_TERMINATED} after {} seconds",
        TEMPLATE_STOP_POLL_ATTEMPTS * TEMPLATE_STOP_POLL_INTERVAL.as_secs() as usize
    ))
}

pub(crate) async fn describe_instance_in_project(
    name: &str,
    account: &str,
    project: &str,
    zone: &str,
) -> Result<InstanceState, String> {
    let _ = account;
    parse_instance_state_value(&gcp::get_instance(project, zone, name).await?)
}

pub(crate) async fn latest_template_snapshot_name(
    account: &str,
    project: &str,
    project_label: &str,
) -> Result<Option<String>, String> {
    Ok(
        list_template_snapshots_in_project(account, project, project_label)
            .await?
            .into_iter()
            .find(|snapshot| snapshot.status == TEMPLATE_SNAPSHOT_STATUS_READY)
            .map(|snapshot| snapshot.name),
    )
}

pub(crate) async fn wait_for_template_snapshot_ready(
    account: &str,
    project: &str,
    snapshot_name: &str,
) -> Result<Snapshot, String> {
    for attempt in 0..TEMPLATE_SNAPSHOT_POLL_ATTEMPTS {
        let snapshot = describe_snapshot_in_project(snapshot_name, account, project).await?;
        if snapshot.status == TEMPLATE_SNAPSHOT_STATUS_READY {
            return Ok(snapshot);
        }

        if attempt + 1 < TEMPLATE_SNAPSHOT_POLL_ATTEMPTS {
            sleep_for(TEMPLATE_SNAPSHOT_POLL_INTERVAL).await;
        }
    }

    Err(format!(
        "template snapshot {snapshot_name} did not reach {TEMPLATE_SNAPSHOT_STATUS_READY} after {} seconds",
        TEMPLATE_SNAPSHOT_POLL_ATTEMPTS * TEMPLATE_SNAPSHOT_POLL_INTERVAL.as_secs() as usize
    ))
}

pub(crate) async fn describe_snapshot_if_exists_in_project(
    snapshot_name: &str,
    account: &str,
    project: &str,
) -> Result<Option<Snapshot>, String> {
    let _ = account;
    match gcp::get_snapshot(project, snapshot_name).await {
        Ok(value) => Ok(Some(parse_snapshot(&value)?)),
        Err(error) if gcloud_resource_was_not_found(&error) => {
            return Ok(None);
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn update_template_snapshot_labels(
    account: &str,
    project: &str,
    snapshot_name: &str,
    updates: &[(&str, &str)],
    removals: &[&str],
) -> Result<(), String> {
    let _ = account;
    let snapshot = gcp::get_snapshot(project, snapshot_name).await?;
    let fingerprint = snapshot
        .get("labelFingerprint")
        .and_then(Value::as_str)
        .ok_or_else(|| "snapshot is missing label fingerprint".to_string())?;
    let mut labels = snapshot
        .get("labels")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in updates {
        labels.insert(
            (*key).to_string(),
            Value::String(sanitize_label_value(value)),
        );
    }
    for key in removals {
        labels.remove(*key);
    }
    gcp::set_snapshot_labels(project, snapshot_name, labels, fingerprint).await?;
    Ok(())
}

async fn describe_snapshot_in_project(
    snapshot_name: &str,
    account: &str,
    project: &str,
) -> Result<Snapshot, String> {
    let _ = account;
    parse_snapshot(&gcp::get_snapshot(project, snapshot_name).await?)
}

pub(crate) async fn list_template_snapshots_in_project(
    account: &str,
    project: &str,
    project_label: &str,
) -> Result<Vec<Snapshot>, String> {
    let _ = account;
    let mut snapshots = parse_snapshots_from_values(gcp::list_snapshots(project).await?)?
        .into_iter()
        .filter(|snapshot| snapshot_matches_project(snapshot, project_label))
        .collect::<Vec<_>>();
    snapshots.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(snapshots)
}

pub(crate) async fn list_template_snapshots() -> Result<Vec<SnapshotTemplate>, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let candidates = candidate_gcloud_configs(&config);

    if candidates.is_empty() {
        return Err("gcloud account and project must be configured".to_string());
    }

    let mut snapshots = Vec::new();
    for gcloud in candidates {
        snapshots.extend(
            list_all_template_snapshots_in_gcloud_project(&gcloud.account, &gcloud.project)
                .await?
                .into_iter()
                .filter_map(snapshot_into_template),
        );
    }

    snapshots.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.project.cmp(&right.project))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(snapshots)
}

async fn list_all_template_snapshots_in_gcloud_project(
    account: &str,
    project: &str,
) -> Result<Vec<Snapshot>, String> {
    let _ = account;
    Ok(
        parse_snapshots_from_values(gcp::list_snapshots(project).await?)?
            .into_iter()
            .filter(|snapshot| snapshot.template && snapshot.project.is_some())
            .collect(),
    )
}

fn parse_workspaces_from_values(entries: Vec<Value>) -> Result<Vec<Workspace>, String> {
    entries.iter().map(parse_workspace).collect()
}

fn parse_snapshots(stdout: &str) -> Result<Vec<Snapshot>, String> {
    let value: Value =
        serde_json::from_str(stdout).map_err(|error| format!("invalid gcloud json: {error}"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| "gcloud did not return a JSON array".to_string())?;

    entries.iter().map(parse_snapshot).collect()
}

fn parse_snapshots_from_values(entries: Vec<Value>) -> Result<Vec<Snapshot>, String> {
    entries.iter().map(parse_snapshot).collect()
}

fn parse_workspace(value: &Value) -> Result<Workspace, String> {
    let name = required_string_field(value, "name")?;
    let created_at = required_string_field(value, "creationTimestamp")?;
    let status = required_string_field(value, "status")?;
    let zone =
        zone_name(value.get("zone")).ok_or_else(|| "workspace is missing zone".to_string())?;
    let labels = value
        .get("labels")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let metadata = parse_instance_metadata(value.get("metadata"));
    let template = labels
        .get("template")
        .and_then(Value::as_str)
        .map(|value| parse_bool_value("template", value))
        .transpose()?
        .unwrap_or(false);

    let project = labels
        .get("project")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let last_active = resolve_workspace_last_active(&metadata);
    let last_working = resolve_workspace_last_working(&metadata);
    let agent_heartbeat_at = resolve_workspace_agent_heartbeat(&metadata);
    let agent_fingerprint = resolve_workspace_agent_fingerprint(&metadata);
    let lifecycle = resolve_workspace_lifecycle(&status, &metadata, agent_heartbeat_at.as_deref());

    let base = WorkspaceBase {
        name,
        project,
        last_active,
        last_working,
        active_session: None,
        created_at,
        status,
        zone,
        lifecycle,
        agent_heartbeat_at,
        agent_fingerprint,
    };

    let unread =
        parse_optional_bool(&metadata, TERMINAL_UNREAD_METADATA_KEY, "unread")?.unwrap_or(false);
    let working = parse_optional_bool(&metadata, TERMINAL_WORKING_METADATA_KEY, "working")?;
    let terminals = Vec::new();
    let browsers = Vec::new();
    let files = Vec::new();

    if template {
        let template_operation = resolve_template_operation(&metadata);
        Ok(Workspace::Template(TemplateWorkspace {
            base,
            unread,
            working,
            terminals,
            browsers,
            files,
            template: true,
            template_operation,
        }))
    } else {
        let branch = metadata.get("branch").cloned().unwrap_or_default();
        let target_branch = metadata.get("target_branch").cloned().unwrap_or_default();

        Ok(Workspace::branch(
            base,
            branch,
            target_branch,
            unread,
            working,
            terminals,
            browsers,
            files,
        ))
    }
}

fn parse_snapshot(value: &Value) -> Result<Snapshot, String> {
    let labels = value
        .get("labels")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let template = labels
        .get("template")
        .and_then(Value::as_str)
        .map(|value| parse_bool_value("template", value))
        .transpose()?
        .unwrap_or(false);

    Ok(Snapshot {
        name: required_string_field(value, "name")?,
        created_at: required_string_field(value, "creationTimestamp")?,
        status: required_string_field(value, "status")?,
        project: labels
            .get("project")
            .and_then(Value::as_str)
            .map(str::to_owned),
        template,
        template_operation_kind: labels
            .get(TEMPLATE_OPERATION_KIND_LABEL_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty()),
        template_operation_phase: labels
            .get(TEMPLATE_OPERATION_PHASE_LABEL_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty()),
        template_operation_id: labels
            .get(TEMPLATE_OPERATION_ID_LABEL_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|value| !value.trim().is_empty()),
    })
}

fn snapshot_into_template(snapshot: Snapshot) -> Option<SnapshotTemplate> {
    Some(SnapshotTemplate {
        name: snapshot.name,
        project: snapshot.project?,
        created_at: snapshot.created_at,
        status: snapshot.status,
    })
}

fn parse_instance_state(stdout: &str) -> Result<InstanceState, String> {
    let value: Value =
        serde_json::from_str(stdout).map_err(|error| format!("invalid gcloud json: {error}"))?;
    parse_instance_state_value(&value)
}

fn parse_instance_state_value(value: &Value) -> Result<InstanceState, String> {
    let status = required_string_field(&value, "status")?;
    let disks = value
        .get("disks")
        .and_then(Value::as_array)
        .ok_or_else(|| "workspace is missing disks".to_string())?;
    let boot_disk = disks
        .iter()
        .find(|disk| disk.get("boot").and_then(Value::as_bool).unwrap_or(false))
        .and_then(|disk| disk.get("source"))
        .and_then(Value::as_str)
        .and_then(resource_name)
        .ok_or_else(|| "workspace is missing boot disk source".to_string())?;

    Ok(InstanceState { status, boot_disk })
}

fn required_string_field(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("workspace is missing {field}"))
}

fn resource_name(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    trimmed.rsplit('/').next().map(str::to_owned)
}

fn zone_name(value: Option<&Value>) -> Option<String> {
    let zone = value?.as_str()?.trim();
    if zone.is_empty() {
        return None;
    }

    zone.rsplit('/').next().map(str::to_owned)
}

fn parse_instance_metadata(metadata: Option<&Value>) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    let Some(items) = metadata
        .and_then(|value| value.get("items"))
        .and_then(Value::as_array)
    else {
        return entries;
    };

    for item in items {
        let Some(key) = item.get("key").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let Some(value) = item.get("value").and_then(Value::as_str) else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        entries.insert(key.to_string(), value.to_string());
    }

    entries
}

fn parse_optional_bool(
    metadata: &HashMap<String, String>,
    key: &str,
    label: &str,
) -> Result<Option<bool>, String> {
    metadata
        .get(key)
        .map(|value| parse_bool_value(label, value))
        .transpose()
}

fn resolve_workspace_last_active(metadata: &HashMap<String, String>) -> Option<String> {
    [
        metadata.get(BROWSER_LAST_ACTIVE_METADATA_KEY),
        metadata.get(FILE_LAST_ACTIVE_METADATA_KEY),
        metadata.get(TERMINAL_LAST_ACTIVE_METADATA_KEY),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .max()
}

fn resolve_workspace_last_working(metadata: &HashMap<String, String>) -> Option<String> {
    metadata.get(TERMINAL_LAST_WORKING_METADATA_KEY).cloned()
}

fn resolve_workspace_lifecycle(
    status: &str,
    metadata: &HashMap<String, String>,
    _agent_heartbeat_at: Option<&str>,
) -> WorkspaceLifecycle {
    let phase = metadata
        .get(WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY)
        .cloned();
    let detail = metadata
        .get(WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let last_error = metadata
        .get(WORKSPACE_LIFECYCLE_ERROR_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let updated_at = metadata
        .get(WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());

    lifecycle_for_status(status, phase, detail, last_error).with_updated_at(updated_at)
}

fn resolve_workspace_agent_heartbeat(metadata: &HashMap<String, String>) -> Option<String> {
    metadata
        .get(WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty())
}

fn resolve_workspace_agent_fingerprint(metadata: &HashMap<String, String>) -> Option<String> {
    metadata
        .get(WORKSPACE_AGENT_FINGERPRINT_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty())
}

fn resolve_template_operation(
    metadata: &HashMap<String, String>,
) -> Option<TemplateOperationState> {
    let kind = metadata
        .get(TEMPLATE_OPERATION_KIND_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let phase = metadata
        .get(TEMPLATE_OPERATION_PHASE_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let detail = metadata
        .get(TEMPLATE_OPERATION_DETAIL_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let last_error = metadata
        .get(TEMPLATE_OPERATION_ERROR_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let updated_at = metadata
        .get(TEMPLATE_OPERATION_UPDATED_AT_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());
    let snapshot_name = metadata
        .get(TEMPLATE_OPERATION_SNAPSHOT_METADATA_KEY)
        .cloned()
        .filter(|value| !value.trim().is_empty());

    Some(TemplateOperationState {
        kind,
        phase,
        detail,
        last_error,
        updated_at,
        snapshot_name,
    })
}

fn parse_bool_value(label: &str, value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("invalid {label} label value: {other}")),
    }
}

pub(crate) fn current_rfc3339_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn lifecycle_for_status(
    status: &str,
    phase: Option<String>,
    detail: Option<String>,
    last_error: Option<String>,
) -> WorkspaceLifecycle {
    let phase = match status {
        "STAGING" | "PROVISIONING" => "provisioning".to_string(),
        "STOPPING" => "stopping".to_string(),
        "SUSPENDING" => "suspending".to_string(),
        "SUSPENDED" => "suspended".to_string(),
        "TERMINATED" | "STOPPED" => "stopped".to_string(),
        "RUNNING" => phase.unwrap_or_else(|| "waiting_for_ssh".to_string()),
        _ => phase.unwrap_or_else(|| status.to_ascii_lowercase()),
    };

    let detail = match phase.as_str() {
        "provisioning" => detail.or_else(|| Some("Provisioning virtual machine".to_string())),
        "waiting_for_ssh" => {
            detail.or_else(|| Some("Waiting for the VM to accept SSH connections".to_string()))
        }
        "bootstrapping" => {
            detail.or_else(|| Some("Preparing repository, credentials, and tools".to_string()))
        }
        "waiting_for_agent" => {
            detail.or_else(|| Some("Waiting for workspace services to come online".to_string()))
        }
        "failed" => detail.or_else(|| Some("Workspace startup failed".to_string())),
        _ => detail,
    };

    WorkspaceLifecycle::new(phase, detail, last_error, None)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn workspace_lifecycle_metadata_entries(
    phase: &str,
    detail: Option<&str>,
    last_error: Option<&str>,
) -> Vec<WorkspaceMetadataEntry> {
    workspace_lifecycle_metadata_entries_with_updated_at(
        phase,
        detail,
        last_error,
        &current_rfc3339_timestamp(),
    )
}

pub(crate) fn workspace_lifecycle_metadata_entries_with_updated_at(
    phase: &str,
    detail: Option<&str>,
    last_error: Option<&str>,
    updated_at: &str,
) -> Vec<WorkspaceMetadataEntry> {
    vec![
        WorkspaceMetadataEntry {
            key: WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
            value: Some(phase.to_string()),
        },
        WorkspaceMetadataEntry {
            key: WORKSPACE_LIFECYCLE_DETAIL_METADATA_KEY.to_string(),
            value: detail.map(sanitize_workspace_metadata_value),
        },
        WorkspaceMetadataEntry {
            key: WORKSPACE_LIFECYCLE_ERROR_METADATA_KEY.to_string(),
            value: last_error.map(sanitize_workspace_metadata_value),
        },
        WorkspaceMetadataEntry {
            key: WORKSPACE_LIFECYCLE_UPDATED_AT_METADATA_KEY.to_string(),
            value: Some(updated_at.to_string()),
        },
    ]
}

pub(crate) fn workspace_lifecycle_state_with_updated_at(
    phase: &str,
    detail: Option<&str>,
    last_error: Option<&str>,
    updated_at: &str,
) -> WorkspaceLifecycle {
    lifecycle_for_status(
        "RUNNING",
        Some(phase.to_string()),
        detail.map(sanitize_workspace_metadata_value),
        last_error.map(sanitize_workspace_metadata_value),
    )
    .with_updated_at(Some(updated_at.to_string()))
}

fn sanitize_workspace_metadata_value(value: &str) -> String {
    let mut sanitized = value.replace(['\r', '\n'], " ");
    for delimiter in METADATA_DELIMITER_CANDIDATES {
        sanitized = sanitized.replace(delimiter, " ");
    }
    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_WORKSPACE_METADATA_VALUE_LEN {
        return collapsed;
    }

    collapsed
        .chars()
        .take(MAX_WORKSPACE_METADATA_VALUE_LEN - 3)
        .collect::<String>()
        + "..."
}

pub(crate) fn workspace_not_ready_error(workspace: &Workspace) -> String {
    let detail = workspace
        .lifecycle()
        .detail()
        .or_else(|| workspace.lifecycle().last_error())
        .unwrap_or(workspace.lifecycle().phase());
    format!("workspace {} is not ready: {detail}", workspace.name())
}

fn workspace_metadata_arg(entries: &[(&str, &str)]) -> Result<String, String> {
    let populated = entries
        .iter()
        .copied()
        .filter(|(_, value)| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if populated.is_empty() {
        return Err("workspace metadata update did not include any values".to_string());
    }

    let delimiter = METADATA_DELIMITER_CANDIDATES
        .iter()
        .copied()
        .find(|delimiter| {
            populated.iter().all(|(key, value)| {
                !key.contains(delimiter)
                    && !value.contains(delimiter)
                    && !key.contains('\n')
                    && !value.contains('\n')
            })
        })
        .ok_or_else(|| {
            "workspace metadata values contain unsupported delimiter content".to_string()
        })?;

    let mut serialized = String::new();
    for (index, (key, value)) in populated.iter().enumerate() {
        if index > 0 {
            serialized.push_str(delimiter);
        }
        serialized.push_str(key);
        serialized.push('=');
        serialized.push_str(value);
    }

    Ok(format!("--metadata=^{delimiter}^{serialized}"))
}

fn compare_workspaces_by_last_active_desc(
    left: &Workspace,
    right: &Workspace,
) -> std::cmp::Ordering {
    match (left.last_active(), right.last_active()) {
        (Some(left_last_active), Some(right_last_active)) => right_last_active
            .cmp(left_last_active)
            .then_with(|| left.name().cmp(right.name())),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.name().cmp(right.name()),
    }
}

pub(crate) fn snapshot_matches_project(snapshot: &Snapshot, project_label: &str) -> bool {
    snapshot.template && snapshot.project.as_deref() == Some(&sanitize_label_value(project_label))
}

fn workspace_into_template(workspace: Workspace) -> Result<TemplateWorkspace, String> {
    match workspace {
        Workspace::Template(workspace) => Ok(workspace),
        Workspace::Branch(workspace) => Err(format!(
            "expected template workspace, found branch workspace {}",
            workspace.base.name
        )),
    }
}

async fn reserve_branch_workspace_identity(
    project: &str,
    account: &str,
    gcloud_project: &str,
    candidates: &[ResolvedGcloudConfig],
) -> Result<(String, String), String> {
    let mut used_names = HashSet::new();
    let mut used_branch_names = HashSet::new();

    for candidate in candidates {
        let existing = list_workspaces_in_project(&candidate.account, &candidate.project).await?;
        let is_target_project = candidate.account == account && candidate.project == gcloud_project;

        for workspace in existing {
            if is_target_project {
                used_names.insert(workspace.name().to_string());
            }
            if let Some(branch) = workspace.branch_name() {
                used_branch_names.insert(branch.to_string());
            }
        }
    }

    select_branch_workspace_identity(project, &used_names, &used_branch_names, random_index())
}

fn select_branch_workspace_identity(
    project: &str,
    used_workspace_names: &HashSet<String>,
    used_branch_names: &HashSet<String>,
    random_index: usize,
) -> Result<(String, String), String> {
    let available = DEFAULT_RIVER_NAMES
        .iter()
        .filter_map(|river| {
            let branch = format!("silo/{river}");
            if used_branch_names.contains(&branch) {
                return None;
            }

            let workspace_name = generate_branch_workspace_name(project, &branch);
            if used_workspace_names.contains(&workspace_name) {
                return None;
            }

            Some((workspace_name, branch))
        })
        .collect::<Vec<_>>();

    if available.is_empty() {
        return Err(format!(
            "no available river names remain for project {}",
            project
        ));
    }

    Ok(available[random_index % available.len()].clone())
}

fn random_index() -> usize {
    (Uuid::new_v4().as_u128() % (usize::MAX as u128 + 1)) as usize
}

fn generate_branch_workspace_name(project: &str, branch: &str) -> String {
    let suffix = branch_workspace_name_suffix(branch);
    generate_project_workspace_name(project, &suffix)
}

pub(crate) fn generate_template_snapshot_name(project: &str) -> String {
    let now = OffsetDateTime::now_utc();
    let suffix = format!("template-{}-{:03}", now.unix_timestamp(), now.millisecond());
    generate_project_workspace_name(project, &suffix)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn generate_template_workspace_name(project: &str) -> String {
    generate_project_workspace_name(project, "template")
}

fn generate_project_workspace_name(project: &str, suffix: &str) -> String {
    let infix = "-silo-";
    let normalized_project = normalize_instance_component(project);
    let normalized_suffix = normalize_instance_component(suffix);
    let max_total_len = 63usize;
    let reserved_len = infix.len();
    let max_project_len = max_total_len
        .saturating_sub(reserved_len + normalized_suffix.len())
        .max(1);
    let truncated_project = truncate_to_boundary(&normalized_project, max_project_len);

    let remaining_suffix_len = max_total_len
        .saturating_sub(truncated_project.len() + reserved_len)
        .max(1);
    let truncated_suffix = truncate_to_boundary(&normalized_suffix, remaining_suffix_len);

    format!("{truncated_project}{infix}{truncated_suffix}")
}

fn branch_workspace_name_suffix(branch: &str) -> String {
    let trimmed = branch.trim();

    if let Some(river) = trimmed.strip_prefix("silo/") {
        return river.to_string();
    }
    if let Some(river) = trimmed.strip_prefix("silo-") {
        return river.to_string();
    }

    trimmed.to_string()
}

fn normalize_instance_component(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_dash = false;

    for ch in value.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_lowercase() || lowered.is_ascii_digit() {
            normalized.push(lowered);
            last_was_dash = false;
        } else if !last_was_dash {
            normalized.push('-');
            last_was_dash = true;
        }
    }

    let normalized = normalized.trim_matches('-');
    let mut cleaned = if normalized.is_empty() {
        "workspace".to_string()
    } else {
        normalized.to_string()
    };

    if !cleaned
        .chars()
        .next()
        .map(|ch| ch.is_ascii_lowercase())
        .unwrap_or(false)
    {
        cleaned.insert(0, 'w');
    }

    while cleaned.ends_with('-') {
        cleaned.pop();
    }

    if cleaned.is_empty() {
        "workspace".to_string()
    } else {
        cleaned
    }
}

fn truncate_to_boundary(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }

    let mut truncated = String::new();
    for ch in value.chars() {
        if truncated.len() + ch.len_utf8() > max_len {
            break;
        }
        truncated.push(ch);
    }

    while truncated.ends_with('-') {
        truncated.pop();
    }

    if truncated.is_empty() {
        "workspace".to_string()
    } else {
        truncated
    }
}

fn sanitize_label_value(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut last_was_dash = false;

    for ch in value.chars() {
        let lowered = ch.to_ascii_lowercase();
        if lowered.is_ascii_lowercase() || lowered.is_ascii_digit() || lowered == '_' {
            sanitized.push(lowered);
            last_was_dash = false;
        } else if !last_was_dash {
            sanitized.push('-');
            last_was_dash = true;
        }
    }

    sanitized.trim_matches('-').to_string()
}

async fn sleep_for(duration: Duration) {
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GcloudConfig, ProjectGcloudConfig, DEFAULT_GCLOUD_DISK_SIZE_GB};
    use indexmap::IndexMap;
    use serde_json::json;

    #[test]
    fn classify_gcloud_resource_error_distinguishes_missing_resources() {
        assert_eq!(
            classify_gcloud_resource_error(
                "failed to describe workspace: The resource 'projects/demo/zones/us-east4-c/instances/demo-silo' was not found"
            ),
            GcloudResourceErrorKind::NotFound
        );
    }

    #[test]
    fn classify_gcloud_resource_error_distinguishes_metadata_fingerprint_conflicts() {
        assert_eq!(
            classify_gcloud_resource_error(
                "failed to update metadata for workspace demo-silo: Could not fetch resource: Supplied fingerprint does not match current metadata fingerprint."
            ),
            GcloudResourceErrorKind::MetadataFingerprintConflict
        );
        assert_eq!(
            classify_gcloud_resource_error(
                "failed to update metadata for workspace demo-silo: permission denied"
            ),
            GcloudResourceErrorKind::Other
        );
    }

    #[test]
    fn workspace_lifecycle_does_not_startup_reconcile_while_updating_agent() {
        let lifecycle = WorkspaceLifecycle::new(
            "updating_workspace_agent",
            Some("Updating workspace observer".to_string()),
            None,
            Some(current_rfc3339_timestamp()),
        );

        assert!(!lifecycle.should_reconcile("RUNNING"));
    }

    fn test_workspace_base(name: &str, last_active: Option<&str>) -> WorkspaceBase {
        WorkspaceBase {
            name: name.to_string(),
            project: Some("demo".to_string()),
            last_active: last_active.map(str::to_string),
            last_working: None,
            active_session: None,
            created_at: "2026-03-11T10:00:00Z".to_string(),
            status: "RUNNING".to_string(),
            zone: "us-east1-b".to_string(),
            lifecycle: WorkspaceLifecycle::new("ready", None, None, None),
            agent_heartbeat_at: None,
            agent_fingerprint: None,
        }
    }

    fn test_branch_workspace(name: &str, last_active: Option<&str>) -> Workspace {
        Workspace::branch(
            test_workspace_base(name, last_active),
            "silo/aare".to_string(),
            String::new(),
            false,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn test_workspace_session(kind: &str, attachment_id: &str) -> WorkspaceSession {
        WorkspaceSession {
            kind: kind.to_string(),
            name: format!("{kind}-{attachment_id}"),
            attachment_id: attachment_id.to_string(),
            path: (kind == "file").then(|| "components.json".to_string()),
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

    fn test_branch_workspace_with_sessions(
        name: &str,
        active_session: Option<WorkspaceActiveSession>,
        terminals: Vec<WorkspaceSession>,
        browsers: Vec<WorkspaceSession>,
        files: Vec<WorkspaceSession>,
    ) -> Workspace {
        let mut base = test_workspace_base(name, None);
        base.active_session = active_session;
        Workspace::branch(
            base,
            "silo/aare".to_string(),
            String::new(),
            false,
            None,
            terminals,
            browsers,
            files,
        )
    }

    #[test]
    fn resolve_gcloud_config_applies_project_overrides() {
        let config = SiloConfig {
            gcloud: GcloudConfig {
                account: "default-account".to_string(),
                service_account: "silo-workspaces@default-project.iam.gserviceaccount.com"
                    .to_string(),
                service_account_key_file: "/Users/test/.silo/default-project-silo-workspaces.json"
                    .to_string(),
                project: "default-project".to_string(),
                region: "us-east4".to_string(),
                zone: "us-east4-c".to_string(),
                machine_type: "e2-standard-4".to_string(),
                disk_size_gb: DEFAULT_GCLOUD_DISK_SIZE_GB,
                disk_type: "pd-ssd".to_string(),
                image_family: "ubuntu".to_string(),
                image_project: "ubuntu-os-cloud".to_string(),
            },
            git: Default::default(),
            codex: Default::default(),
            claude: Default::default(),
            projects: IndexMap::new(),
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: String::new(),
            env_files: Vec::new(),
            gcloud: ProjectGcloudConfig {
                project: Some("override-project".to_string()),
                region: Some("us-west1".to_string()),
                zone: Some("us-west1-b".to_string()),
                machine_type: Some("n2-standard-8".to_string()),
                disk_size_gb: Some(120),
                ..Default::default()
            },
        };

        let resolved = resolve_gcloud_config(&config, &project);

        assert_eq!(
            resolved.account,
            "silo-workspaces@default-project.iam.gserviceaccount.com"
        );
        assert_eq!(
            resolved.service_account,
            "silo-workspaces@default-project.iam.gserviceaccount.com"
        );
        assert_eq!(resolved.project, "override-project");
        assert_eq!(resolved.region, "us-west1");
        assert_eq!(resolved.zone, "us-west1-b");
        assert_eq!(resolved.machine_type, "n2-standard-8");
        assert_eq!(resolved.disk_size_gb, 120);
        assert_eq!(resolved.disk_type, "pd-ssd");
    }

    #[test]
    fn resolve_gcloud_config_ignores_project_account_override_when_service_account_is_set() {
        let config = SiloConfig {
            gcloud: GcloudConfig {
                account: "default-account".to_string(),
                service_account: "silo-workspaces@default-project.iam.gserviceaccount.com"
                    .to_string(),
                service_account_key_file: "/Users/test/.silo/default-project-silo-workspaces.json"
                    .to_string(),
                project: "default-project".to_string(),
                region: "us-east4".to_string(),
                zone: "us-east4-c".to_string(),
                machine_type: "e2-standard-4".to_string(),
                disk_size_gb: DEFAULT_GCLOUD_DISK_SIZE_GB,
                disk_type: "pd-ssd".to_string(),
                image_family: "ubuntu".to_string(),
                image_project: "ubuntu-os-cloud".to_string(),
            },
            git: Default::default(),
            codex: Default::default(),
            claude: Default::default(),
            projects: IndexMap::new(),
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: String::new(),
            env_files: Vec::new(),
            gcloud: ProjectGcloudConfig {
                account: Some("someone-else@example.com".to_string()),
                ..Default::default()
            },
        };

        let resolved = resolve_gcloud_config(&config, &project);

        assert_eq!(
            resolved.account,
            "silo-workspaces@default-project.iam.gserviceaccount.com"
        );
        assert_eq!(
            resolved.service_account,
            "silo-workspaces@default-project.iam.gserviceaccount.com"
        );
    }

    #[test]
    fn create_workspace_args_include_service_account_when_configured() {
        let gcloud = ResolvedGcloudConfig {
            account: "acct".to_string(),
            service_account: "silo-workspaces@proj.iam.gserviceaccount.com".to_string(),
            project: "proj".to_string(),
            region: "us-east4".to_string(),
            zone: "us-east4-c".to_string(),
            machine_type: "e2-standard-4".to_string(),
            disk_size_gb: 80,
            disk_type: "pd-ssd".to_string(),
            image_family: "ubuntu-2404-lts-amd64".to_string(),
            image_project: "ubuntu-os-cloud".to_string(),
        };

        let args = create_workspace_args(
            "ws-demo-abc",
            "Demo Project",
            "Aare",
            "Feature/Inbox",
            &WorkspaceBootSource::ImageFamily,
            &gcloud,
        );

        assert!(args.contains(&"--zone=us-east4-c".to_string()));
        assert!(args.contains(&"--machine-type=e2-standard-4".to_string()));
        assert!(args.contains(&"--boot-disk-size=80GB".to_string()));
        assert!(args.contains(&"--boot-disk-type=pd-ssd".to_string()));
        assert!(args.contains(&"--image-family=ubuntu-2404-lts-amd64".to_string()));
        assert!(args.contains(&"--image-project=ubuntu-os-cloud".to_string()));
        assert!(args.contains(&"--async".to_string()));
        assert!(args.contains(&"--labels=project=demo-project".to_string()));
        assert!(args.iter().any(|arg| arg.contains("branch=Aare")));
        assert!(args
            .iter()
            .any(|arg| arg.contains("target_branch=Feature/Inbox")));
        assert!(args
            .iter()
            .any(|arg| arg.contains("workspace-lifecycle-phase=provisioning")));
        assert!(args.contains(
            &"--service-account=silo-workspaces@proj.iam.gserviceaccount.com".to_string()
        ));
        assert!(args.contains(&"--scopes=https://www.googleapis.com/auth/compute".to_string()));
    }

    #[test]
    fn create_workspace_args_use_snapshot_when_available() {
        let gcloud = ResolvedGcloudConfig {
            account: "acct".to_string(),
            service_account: "silo-workspaces@proj.iam.gserviceaccount.com".to_string(),
            project: "proj".to_string(),
            region: "us-east4".to_string(),
            zone: "us-east4-c".to_string(),
            machine_type: "e2-standard-4".to_string(),
            disk_size_gb: 80,
            disk_type: "pd-ssd".to_string(),
            image_family: "ubuntu-2404-lts-amd64".to_string(),
            image_project: "ubuntu-os-cloud".to_string(),
        };

        let args = create_workspace_args(
            "ws-demo-abc",
            "Demo Project",
            "Aare",
            "",
            &WorkspaceBootSource::Snapshot("demo-silo-template-1710000000-123".to_string()),
            &gcloud,
        );

        assert!(args.contains(&"--source-snapshot=demo-silo-template-1710000000-123".to_string()));
        assert!(!args.iter().any(|arg| arg.starts_with("--image-family=")));
        assert!(!args.iter().any(|arg| arg.starts_with("--image-project=")));
        assert!(args.contains(&"--labels=project=demo-project".to_string()));
        assert!(args.iter().any(|arg| arg.contains("branch=Aare")));
    }

    #[test]
    fn create_workspace_args_disable_vm_identity_without_service_account() {
        let gcloud = ResolvedGcloudConfig {
            account: "acct".to_string(),
            service_account: String::new(),
            project: "proj".to_string(),
            region: "us-east4".to_string(),
            zone: "us-east4-c".to_string(),
            machine_type: "e2-standard-4".to_string(),
            disk_size_gb: 80,
            disk_type: "pd-ssd".to_string(),
            image_family: "ubuntu-2404-lts-amd64".to_string(),
            image_project: "ubuntu-os-cloud".to_string(),
        };

        let args = create_workspace_args(
            "ws-demo-abc",
            "Demo Project",
            "Aare",
            "",
            &WorkspaceBootSource::ImageFamily,
            &gcloud,
        );

        assert!(args.contains(&"--async".to_string()));
        assert!(args.contains(&"--no-service-account".to_string()));
        assert!(args.contains(&"--no-scopes".to_string()));
        assert!(args.contains(&"--labels=project=demo-project".to_string()));
        assert!(args.iter().any(|arg| arg.contains("branch=Aare")));
    }

    #[test]
    fn create_template_workspace_args_use_template_label_and_image() {
        let gcloud = ResolvedGcloudConfig {
            account: "acct".to_string(),
            service_account: "silo-workspaces@proj.iam.gserviceaccount.com".to_string(),
            project: "proj".to_string(),
            region: "us-east4".to_string(),
            zone: "us-east4-c".to_string(),
            machine_type: "e2-standard-4".to_string(),
            disk_size_gb: 80,
            disk_type: "pd-ssd".to_string(),
            image_family: "ubuntu-2404-lts-amd64".to_string(),
            image_project: "ubuntu-os-cloud".to_string(),
        };

        let args = create_template_workspace_args("demo-silo-template", "Demo", None, &gcloud);

        assert!(args.contains(&"--image-family=ubuntu-2404-lts-amd64".to_string()));
        assert!(args.contains(&"--image-project=ubuntu-os-cloud".to_string()));
        assert!(args.contains(&"--labels=project=demo,template=true".to_string()));
        assert!(args
            .iter()
            .any(|arg| arg.contains("workspace-lifecycle-phase=provisioning")));
        assert!(!args.iter().any(|arg| arg.starts_with("--source-snapshot=")));
    }

    #[test]
    fn generate_branch_workspace_name_uses_project_and_river() {
        let name = generate_branch_workspace_name("Lenny", "silo/nile");

        assert_eq!(name, "lenny-silo-nile");
    }

    #[test]
    fn select_branch_workspace_identity_skips_globally_used_rivers() {
        let used_workspace_names = HashSet::new();
        let used_branch_names = ["silo/aabach", "silo/aach"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>();

        let (workspace_name, branch_name) =
            select_branch_workspace_identity("Lenny", &used_workspace_names, &used_branch_names, 0)
                .expect("expected an available river");

        assert_eq!(branch_name, "silo/aalbach");
        assert_eq!(workspace_name, "lenny-silo-aalbach");
    }

    #[test]
    fn select_branch_workspace_identity_skips_target_name_collisions() {
        let used_workspace_names = ["lenny-silo-aabach"]
            .into_iter()
            .map(str::to_string)
            .collect::<HashSet<_>>();
        let used_branch_names = HashSet::new();

        let (workspace_name, branch_name) =
            select_branch_workspace_identity("Lenny", &used_workspace_names, &used_branch_names, 0)
                .expect("expected an available river");

        assert_eq!(branch_name, "silo/aach");
        assert_eq!(workspace_name, "lenny-silo-aach");
    }

    #[test]
    fn select_branch_workspace_identity_uses_random_index_with_available_rivers() {
        let used_workspace_names = HashSet::new();
        let used_branch_names = HashSet::new();

        let (workspace_name, branch_name) =
            select_branch_workspace_identity("Lenny", &used_workspace_names, &used_branch_names, 1)
                .expect("expected an available river");

        assert_eq!(branch_name, "silo/aach");
        assert_eq!(workspace_name, "lenny-silo-aach");
    }

    #[test]
    fn generate_template_workspace_name_uses_template_suffix() {
        let name = generate_template_workspace_name("Lenny");

        assert_eq!(name, "lenny-silo-template");
    }

    #[test]
    fn add_workspace_metadata_args_adds_metadata_when_value_present() {
        let args = add_workspace_metadata_args(
            &test_branch_workspace("ws-demo-123", None),
            &[("target_branch", "Feature/Inbox")],
        )
        .expect("metadata args should build");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "add-metadata".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--metadata=^|^target_branch=Feature/Inbox".to_string(),
            ]
        );
    }

    #[test]
    fn workspace_lifecycle_metadata_entries_sanitize_multiline_errors() {
        let entries = workspace_lifecycle_metadata_entries(
            "failed",
            Some("Workspace startup failed\nretrying"),
            Some("line 1\nline 2 | @@ SILO_METADATA_DELIM"),
        );
        let detail = entries[1]
            .value
            .as_deref()
            .expect("detail should be present");
        let error = entries[2]
            .value
            .as_deref()
            .expect("error should be present");

        assert_eq!(detail, "Workspace startup failed retrying");
        assert_eq!(error, "line 1 line 2");
    }

    #[test]
    fn resolve_workspace_lifecycle_keeps_waiting_for_agent_with_fresh_heartbeat() {
        let mut metadata = HashMap::new();
        metadata.insert(
            WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
            "waiting_for_agent".to_string(),
        );
        metadata.insert(
            WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY.to_string(),
            (OffsetDateTime::now_utc() - time::Duration::seconds(10))
                .format(&Rfc3339)
                .expect("heartbeat timestamp should format"),
        );

        let lifecycle = resolve_workspace_lifecycle(
            "RUNNING",
            &metadata,
            metadata
                .get(WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY)
                .map(String::as_str),
        );

        assert_eq!(lifecycle.phase(), "waiting_for_agent");
        assert_eq!(
            lifecycle.detail(),
            Some("Waiting for workspace services to come online")
        );
    }

    #[test]
    fn resolve_workspace_lifecycle_keeps_explicit_ready_phase() {
        let mut metadata = HashMap::new();
        metadata.insert(
            WORKSPACE_LIFECYCLE_PHASE_METADATA_KEY.to_string(),
            "ready".to_string(),
        );

        let lifecycle = resolve_workspace_lifecycle("RUNNING", &metadata, None);

        assert!(lifecycle.is_ready());
        assert_eq!(lifecycle.phase(), "ready");
    }

    #[test]
    fn remove_workspace_metadata_args_removes_metadata_when_value_empty() {
        let args = remove_workspace_metadata_args(
            &test_branch_workspace("ws-demo-123", None),
            &["branch"],
        );

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "remove-metadata".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--keys=branch".to_string(),
            ]
        );
    }

    #[test]
    fn stop_workspace_args_run_async() {
        let args = stop_workspace_args("ws-demo-123", "us-east1-b");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "stop".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--async".to_string(),
            ]
        );
    }

    #[test]
    fn suspend_workspace_args_run_async() {
        let args = suspend_workspace_args("ws-demo-123", "us-east1-b");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "suspend".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--async".to_string(),
            ]
        );
    }

    #[test]
    fn resume_workspace_args_run_async() {
        let args = resume_workspace_args("ws-demo-123", "us-east1-b");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "resume".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--async".to_string(),
            ]
        );
    }

    #[test]
    fn create_template_snapshot_args_use_source_disk_and_zone() {
        let args =
            create_template_snapshot_args("demo-silo-template-123", "disk-1", "us-east1-b", "Demo");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "snapshots".to_string(),
                "create".to_string(),
                "demo-silo-template-123".to_string(),
                "--source-disk=disk-1".to_string(),
                "--source-disk-zone=us-east1-b".to_string(),
                "--labels=project=demo,template=true".to_string(),
                "--async".to_string(),
            ]
        );
    }

    #[test]
    fn delete_snapshot_args_delete_quietly() {
        let args = delete_snapshot_args("demo-silo-template-123");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "snapshots".to_string(),
                "delete".to_string(),
                "demo-silo-template-123".to_string(),
                "--quiet".to_string(),
            ]
        );
    }

    #[test]
    fn delete_workspace_args_run_quiet_without_async() {
        let args = delete_workspace_args("ws-demo-123", "us-east1-b");

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "delete".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--quiet".to_string(),
            ]
        );
    }

    #[test]
    fn parse_workspace_maps_labels_and_created_at() {
        let workspace = parse_workspace(&json!({
            "name": "ws-demo-123",
            "zone": "https://www.googleapis.com/compute/v1/projects/test/zones/us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "labels": {
                "project": "demo",
            },
            "metadata": {
                "items": [
                    { "key": "branch", "value": "silo/aare" },
                    { "key": "target_branch", "value": "main" },
                    { "key": "terminal-unread", "value": "true" },
                    { "key": "terminal-working", "value": "true" },
                    { "key": "terminal-last-active", "value": "2026-03-11T13:05:00Z" },
                    { "key": "terminal-last-working", "value": "2026-03-11T13:04:30Z" },
                    { "key": "terminal-session-terminal-1", "value": "{\"type\":\"terminal\",\"name\":\"codex\",\"attachment_id\":\"terminal-1\",\"working\":true,\"unread\":false}" }
                ]
            }
        }))
        .expect("workspace should parse");

        let Workspace::Branch(workspace) = workspace else {
            panic!("workspace should parse as a branch workspace");
        };

        assert_eq!(workspace.base.name, "ws-demo-123");
        assert_eq!(workspace.base.project.as_deref(), Some("demo"));
        assert_eq!(workspace.branch, "silo/aare");
        assert_eq!(workspace.target_branch, "main");
        assert!(workspace.unread);
        assert_eq!(workspace.working, Some(true));
        assert_eq!(
            workspace.base.last_active.as_deref(),
            Some("2026-03-11T13:05:00Z")
        );
        assert_eq!(
            workspace.base.last_working.as_deref(),
            Some("2026-03-11T13:04:30Z")
        );
        assert_eq!(workspace.base.created_at, "2026-03-11T13:00:00.000-04:00");
        assert_eq!(workspace.base.zone, "us-east1-b");
        assert_eq!(workspace.base.active_session, None);
        assert!(workspace.terminals.is_empty());
    }

    #[test]
    fn parse_workspace_ignores_flat_session_metadata() {
        let workspace = parse_workspace(&json!({
            "name": "ws-demo-123",
            "zone": "us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "labels": {
                "project": "demo",
            },
            "metadata": {
                "items": [
                    { "key": "branch", "value": "feature/inbox" },
                    { "key": "target_branch", "value": "main" },
                    { "key": "terminal-working", "value": "true" },
                    { "key": "terminal-unread", "value": "true" },
                    { "key": "terminal-session-terminal-1", "value": "{\"type\":\"terminal\",\"name\":\"codex\",\"attachment_id\":\"terminal-1\",\"working\":true,\"unread\":false}" }
                ]
            }
        }))
        .expect("workspace should parse");

        let Workspace::Branch(workspace) = workspace else {
            panic!("workspace should parse as a branch workspace");
        };

        assert_eq!(workspace.branch, "feature/inbox");
        assert_eq!(workspace.target_branch, "main");
        assert!(workspace.unread);
        assert_eq!(workspace.working, Some(true));
        assert!(workspace.terminals.is_empty());
    }

    #[test]
    fn parse_workspace_defaults_missing_branch_and_target_branch_labels() {
        let workspace = parse_workspace(&json!({
            "name": "ws-demo-123",
            "zone": "us-east1-b",
            "status": "TERMINATED",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00"
        }))
        .expect("workspace should parse");

        let Workspace::Branch(workspace) = workspace else {
            panic!("workspace should default to a branch workspace");
        };

        assert_eq!(workspace.base.project, None);
        assert_eq!(workspace.branch, "");
        assert_eq!(workspace.target_branch, "");
        assert!(!workspace.unread);
        assert_eq!(workspace.working, None);
        assert_eq!(workspace.base.last_active, None);
        assert_eq!(workspace.base.last_working, None);
        assert_eq!(workspace.base.active_session, None);
    }

    #[test]
    fn parse_workspace_ignores_invalid_session_rows() {
        let workspace = parse_workspace(&json!({
            "name": "ws-demo-123",
            "zone": "us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "metadata": {
                "items": [
                    { "key": "branch", "value": "silo/aare" },
                    { "key": "target_branch", "value": "main" },
                    { "key": "terminal-session-terminal-1", "value": "{\"type\":\"terminal\",\"name\":\"good\",\"attachment_id\":\"terminal-1\"}" },
                    { "key": "terminal-session-terminal-2", "value": "{\"type\":\"terminal\",\"name\":\"\",\"attachment_id\":\"terminal-2\"}" }
                ]
            }
        }))
        .expect("workspace should parse");

        let Workspace::Branch(workspace) = workspace else {
            panic!("workspace should parse as a branch workspace");
        };

        assert_eq!(workspace.branch, "silo/aare");
        assert_eq!(workspace.target_branch, "main");
        assert!(workspace.terminals.is_empty());
    }

    #[test]
    fn parse_workspace_maps_template_labels() {
        let workspace = parse_workspace(&json!({
            "name": "template-demo-123",
            "zone": "us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "labels": {
                "project": "demo",
                "template": "true"
            }
        }))
        .expect("workspace should parse");

        let Workspace::Template(workspace) = workspace else {
            panic!("workspace should parse as a template workspace");
        };

        assert!(workspace.template);
        assert_eq!(workspace.base.lifecycle.phase(), "waiting_for_ssh");
        assert_eq!(workspace.base.name, "template-demo-123");
        assert_eq!(workspace.base.project.as_deref(), Some("demo"));
        assert!(!workspace.unread);
        assert_eq!(workspace.working, None);
        assert!(workspace.terminals.is_empty());
        assert!(workspace.browsers.is_empty());
    }

    #[test]
    fn parse_workspace_maps_template_session_metadata() {
        let workspace = parse_workspace(&json!({
            "name": "template-demo-123",
            "zone": "us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "labels": {
                "project": "demo",
                "template": "true"
            },
            "metadata": {
                "items": [
                    { "key": "workspace-lifecycle-phase", "value": "ready" },
                    { "key": "terminal-last-active", "value": "2026-03-16T04:18:39Z" },
                    { "key": "terminal-working", "value": "false" },
                    { "key": "terminal-unread", "value": "false" },
                    { "key": "terminal-session-terminal-1773634611341", "value": "{\"type\":\"terminal\",\"name\":\"bun run build:render\",\"attachment_id\":\"terminal-1773634611341\"}" }
                ]
            },
        }))
        .expect("workspace should parse");

        let Workspace::Template(workspace) = workspace else {
            panic!("workspace should parse as a template workspace");
        };

        assert!(workspace.template);
        assert!(workspace.base.lifecycle.is_ready());
        assert_eq!(
            workspace.base.last_active.as_deref(),
            Some("2026-03-16T04:18:39Z")
        );
        assert!(!workspace.unread);
        assert_eq!(workspace.working, Some(false));
        assert!(workspace.terminals.is_empty());
    }

    #[test]
    fn parse_workspace_requires_both_active_session_metadata_keys() {
        let workspace = parse_workspace(&json!({
            "name": "ws-demo-123",
            "zone": "us-east1-b",
            "status": "RUNNING",
            "creationTimestamp": "2026-03-11T13:00:00.000-04:00",
            "metadata": {
                "items": [
                    { "key": "active-session-kind", "value": "terminal" }
                ]
            }
        }))
        .expect("workspace should parse");

        let Workspace::Branch(workspace) = workspace else {
            panic!("workspace should parse as a branch workspace");
        };

        assert_eq!(workspace.base.active_session, None);
    }

    #[test]
    fn clear_invalid_workspace_active_session_removes_stale_active_session() {
        let workspace = overlay_workspace_active_session(
            test_branch_workspace("ws-demo-123", None),
            Some(WorkspaceActiveSession::new("terminal", "terminal-1")),
        );

        let workspace = clear_invalid_workspace_active_session(workspace);

        assert_eq!(workspace.active_session(), None);
    }

    #[test]
    fn metadata_only_observation_does_not_drop_session_remove_tombstone() {
        let manager = crate::state::WorkspaceMetadataManager::default();
        manager.remove_workspace_session("ws-demo-123", "file", "file-1");

        let metadata_workspace =
            test_branch_workspace_with_sessions("ws-demo-123", None, vec![], vec![], vec![]);
        let runtime_workspace = test_branch_workspace_with_sessions(
            "ws-demo-123",
            None,
            vec![],
            vec![],
            vec![test_workspace_session("file", "file-1")],
        );

        manager.reconcile_workspace_observation(&metadata_workspace, None);
        let projected = manager.apply_workspace_state(runtime_workspace);

        assert!(!projected.has_session("file", "file-1"));
    }

    #[test]
    fn session_remove_tombstone_clears_after_metadata_and_runtime_absent() {
        let manager = crate::state::WorkspaceMetadataManager::default();
        manager.remove_workspace_session("ws-demo-123", "file", "file-1");

        let metadata_workspace =
            test_branch_workspace_with_sessions("ws-demo-123", None, vec![], vec![], vec![]);
        let runtime_workspace = test_branch_workspace_with_sessions(
            "ws-demo-123",
            None,
            vec![],
            vec![],
            vec![test_workspace_session("file", "file-1")],
        );

        manager.reconcile_workspace_observation(&metadata_workspace, Some(&runtime_workspace));
        let projected_while_runtime_stale =
            manager.apply_workspace_state(runtime_workspace.clone());
        assert!(!projected_while_runtime_stale.has_session("file", "file-1"));

        manager.reconcile_workspace_observation(&metadata_workspace, Some(&metadata_workspace));
        let projected_after_convergence = manager.apply_workspace_state(runtime_workspace);
        assert!(projected_after_convergence.has_session("file", "file-1"));
    }

    #[test]
    fn metadata_only_observation_does_not_drop_session_upsert_overlay() {
        let manager = crate::state::WorkspaceMetadataManager::default();
        manager.upsert_workspace_session("ws-demo-123", test_workspace_session("file", "file-1"));

        let metadata_workspace = test_branch_workspace_with_sessions(
            "ws-demo-123",
            None,
            vec![],
            vec![],
            vec![test_workspace_session("file", "file-1")],
        );
        let runtime_workspace =
            test_branch_workspace_with_sessions("ws-demo-123", None, vec![], vec![], vec![]);

        manager.reconcile_workspace_observation(&metadata_workspace, None);
        let projected = manager.apply_workspace_state(runtime_workspace);

        assert!(projected.has_session("file", "file-1"));
    }

    #[test]
    fn session_upsert_overlay_clears_after_metadata_and_runtime_match() {
        let manager = crate::state::WorkspaceMetadataManager::default();
        manager.upsert_workspace_session("ws-demo-123", test_workspace_session("file", "file-1"));

        let observed_workspace = test_branch_workspace_with_sessions(
            "ws-demo-123",
            None,
            vec![],
            vec![],
            vec![test_workspace_session("file", "file-1")],
        );
        let stale_runtime_workspace =
            test_branch_workspace_with_sessions("ws-demo-123", None, vec![], vec![], vec![]);

        manager.reconcile_workspace_observation(&observed_workspace, Some(&observed_workspace));
        let projected_after_convergence = manager.apply_workspace_state(stale_runtime_workspace);

        assert!(!projected_after_convergence.has_session("file", "file-1"));
    }

    #[test]
    fn replace_workspace_terminals_replaces_only_terminal_sessions() {
        let mut overlay = HashMap::new();
        overlay.insert(
            "terminal:terminal-1".to_string(),
            Some(WorkspaceSession {
                kind: "terminal".to_string(),
                name: "shell".to_string(),
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
            }),
        );
        overlay.insert(
            "browser:browser-1".to_string(),
            Some(WorkspaceSession {
                kind: "browser".to_string(),
                name: "Docs".to_string(),
                attachment_id: "browser-1".to_string(),
                path: None,
                url: Some("https://example.com".to_string()),
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            }),
        );
        overlay.insert(
            "file:file-1".to_string(),
            Some(WorkspaceSession {
                kind: "file".to_string(),
                name: "src/main.rs".to_string(),
                attachment_id: "file-1".to_string(),
                path: Some("src/main.rs".to_string()),
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            }),
        );
        let workspace =
            overlay_workspace_sessions(test_branch_workspace("ws-demo-123", None), &overlay);

        let workspace = replace_workspace_terminals(
            workspace,
            vec![WorkspaceSession {
                kind: "terminal".to_string(),
                name: "codex".to_string(),
                attachment_id: "terminal-2".to_string(),
                path: None,
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: Some(true),
                unread: Some(false),
            }],
        );

        assert_eq!(workspace.terminals().len(), 1);
        assert_eq!(workspace.terminals()[0].attachment_id, "terminal-2");
        assert_eq!(workspace.terminals()[0].name, "codex");
        assert_eq!(workspace.browsers().len(), 1);
        assert_eq!(workspace.browsers()[0].attachment_id, "browser-1");
        assert_eq!(workspace.files().len(), 1);
        assert_eq!(workspace.files()[0].attachment_id, "file-1");
    }

    #[test]
    fn parse_instance_state_extracts_status_and_boot_disk() {
        let instance = parse_instance_state(
            &json!({
                "status": "TERMINATED",
                "disks": [
                    {
                        "boot": true,
                        "source": "https://www.googleapis.com/compute/v1/projects/test/zones/us-east1-b/disks/demo-disk"
                    }
                ]
            })
            .to_string(),
        )
        .expect("instance should parse");

        assert_eq!(instance.status, "TERMINATED");
        assert_eq!(instance.boot_disk, "demo-disk");
    }

    #[test]
    fn parse_snapshots_maps_snapshot_fields() {
        let snapshots = parse_snapshots(
            &json!([
                {
                    "name": "demo-silo-template-123",
                    "status": "READY",
                    "creationTimestamp": "2026-03-12T12:00:00Z",
                    "labels": {
                        "project": "demo",
                        "template": "true"
                    }
                }
            ])
            .to_string(),
        )
        .expect("snapshots should parse");

        assert_eq!(
            snapshots,
            vec![Snapshot {
                name: "demo-silo-template-123".to_string(),
                status: "READY".to_string(),
                created_at: "2026-03-12T12:00:00Z".to_string(),
                project: Some("demo".to_string()),
                template: true,
                template_operation_kind: None,
                template_operation_phase: None,
                template_operation_id: None,
            }]
        );
    }

    #[test]
    fn snapshot_into_template_requires_project_label() {
        let snapshot = Snapshot {
            name: "demo-silo-template-123".to_string(),
            status: "READY".to_string(),
            created_at: "2026-03-12T12:00:00Z".to_string(),
            project: None,
            template: true,
            template_operation_kind: None,
            template_operation_phase: None,
            template_operation_id: None,
        };

        assert_eq!(snapshot_into_template(snapshot), None);
    }

    #[test]
    fn snapshot_matches_project_uses_template_prefix() {
        let matching = Snapshot {
            name: "demo-silo-template-1710000000-123".to_string(),
            status: "READY".to_string(),
            created_at: "2026-03-12T12:00:00Z".to_string(),
            project: Some("demo".to_string()),
            template: true,
            template_operation_kind: None,
            template_operation_phase: None,
            template_operation_id: None,
        };
        let non_matching = Snapshot {
            name: "other-silo-template-1710000000-123".to_string(),
            status: "READY".to_string(),
            created_at: "2026-03-12T12:00:00Z".to_string(),
            project: Some("other".to_string()),
            template: true,
            template_operation_kind: None,
            template_operation_phase: None,
            template_operation_id: None,
        };

        assert!(snapshot_matches_project(&matching, "Demo"));
        assert!(!snapshot_matches_project(&non_matching, "Demo"));
    }

    #[test]
    fn generate_template_snapshot_name_uses_template_prefix_and_timestamp_suffix() {
        let name = generate_template_snapshot_name("Demo");

        assert!(name.starts_with("demo-silo-template-"));
        assert!(name.len() <= 63);
        assert!(name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'));
    }

    #[test]
    fn compare_workspaces_sorts_last_active_desc_with_nulls_last() {
        let mut workspaces = vec![
            test_branch_workspace("c", None),
            test_branch_workspace("b", Some("2026-03-11T11:00:00Z")),
            test_branch_workspace("a", Some("2026-03-11T12:00:00Z")),
        ];

        workspaces.sort_by(compare_workspaces_by_last_active_desc);

        assert_eq!(
            workspaces
                .iter()
                .map(|workspace| workspace.name())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn generate_template_workspace_name_normalizes_and_truncates() {
        let name = generate_template_workspace_name(
            "123 Very Loud Project Name With Spaces And Symbols!!!",
        );

        assert!(name.starts_with("w123-"));
        assert!(name.contains("-silo-"));
        assert!(name.ends_with("template"));
        assert!(name.len() <= 63);
        assert!(name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'));
    }

    #[test]
    fn trim_prompt_input_rejects_blank_values() {
        assert_eq!(
            trim_prompt_input("   ").expect_err("blank prompt should fail"),
            "prompt must not be empty"
        );
    }

    #[test]
    fn assistant_provider_for_model_rejects_unknown_model() {
        assert_eq!(
            assistant_provider_for_model("gpt").expect_err("unknown model should fail"),
            "unsupported assistant model: gpt"
        );
    }

    #[test]
    fn assistant_provider_for_model_parses_claude() {
        assert_eq!(
            assistant_provider_for_model("claude").expect("claude model should parse"),
            terminal::AssistantProvider::Claude
        );
    }
}
