use crate::config::{ConfigStore, ProjectConfig, SiloConfig};
use crate::river_names::DEFAULT_RIVER_NAMES;
use crate::state::{
    active_session_metadata_entries, workspace_last_active_metadata_key, WorkspaceMetadataEntry,
    ACTIVE_SESSION_ATTACHMENT_ID_METADATA_KEY, ACTIVE_SESSION_KIND_METADATA_KEY,
    BROWSER_LAST_ACTIVE_METADATA_KEY, BROWSER_SESSION_METADATA_PREFIX,
    TERMINAL_LAST_ACTIVE_METADATA_KEY, TERMINAL_LAST_WORKING_METADATA_KEY,
    TERMINAL_SESSION_METADATA_PREFIX, TERMINAL_UNREAD_METADATA_KEY, TERMINAL_WORKING_METADATA_KEY,
};
use crate::terminal;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::State;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TemplateWorkspace {
    #[serde(flatten)]
    base: WorkspaceBase,
    unread: bool,
    working: Option<bool>,
    terminals: Vec<WorkspaceSession>,
    browsers: Vec<WorkspaceSession>,
    template: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SnapshotTemplate {
    name: String,
    project: String,
    created_at: String,
    status: String,
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
    ready: bool,
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
    ) -> Self {
        Self::Branch(BranchWorkspace {
            base,
            branch,
            target_branch,
            unread,
            working,
            terminals,
            browsers,
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

    pub(crate) fn ready(&self) -> bool {
        self.base().ready
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

    pub(crate) fn sessions(&self) -> Vec<WorkspaceSession> {
        let mut sessions = self.terminals().to_vec();
        sessions.extend_from_slice(self.browsers());
        terminal::sort_workspace_sessions_oldest_to_newest(&mut sessions);
        sessions
    }

    pub(crate) fn has_session(&self, kind: &str, attachment_id: &str) -> bool {
        self.sessions()
            .into_iter()
            .any(|session| session.kind == kind && session.attachment_id == attachment_id)
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
    for session in sessions.into_values() {
        if session.kind == "browser" {
            browsers.push(session);
        } else {
            terminals.push(session);
        }
    }
    terminal::sort_workspace_sessions_oldest_to_newest(&mut terminals);
    terminal::sort_workspace_sessions_oldest_to_newest(&mut browsers);
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
            Workspace::Branch(workspace)
        }
        Workspace::Template(mut workspace) => {
            workspace.unread = unread;
            workspace.working = working;
            workspace.terminals = terminals;
            workspace.browsers = browsers;
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

#[derive(Debug)]
pub(crate) struct CommandResult {
    pub(crate) success: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceLookup {
    pub(crate) workspace: Workspace,
    pub(crate) account: String,
    pub(crate) gcloud_project: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstanceState {
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

    Ok(state.apply_workspace_states(workspaces))
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

    let result = run_gcloud(
        &gcloud.account,
        &gcloud.project,
        create_workspace_args(
            &workspace_name,
            &project,
            &branch_name,
            &project_config.target_branch,
            &boot_source,
            &gcloud,
        ),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to create workspace", &result.stderr));
    }

    log::info!("workspace {workspace_name} creation started for project {project}");
    terminal::start_workspace_ssh_readiness(workspace_name.clone());
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

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        [
            "compute".to_string(),
            "instances".to_string(),
            "start".to_string(),
            workspace.clone(),
            format!("--zone={}", lookup.workspace.zone()),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to start workspace", &result.stderr));
    }

    update_workspace_label_in_lookup(lookup.clone(), "ready", "false").await?;
    terminal::start_workspace_ssh_readiness(workspace.clone());

    log::info!("workspace {} started", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_resume_workspace(workspace: String) -> Result<(), String> {
    log::info!("resuming workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        resume_workspace_args(&workspace, lookup.workspace.zone()),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to resume workspace", &result.stderr));
    }

    update_workspace_label_in_lookup(lookup.clone(), "ready", "false").await?;
    terminal::start_workspace_ssh_readiness(workspace.clone());

    log::info!("workspace {} resume initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_stop_workspace(workspace: String) -> Result<(), String> {
    log::info!("stopping workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        stop_workspace_args(&workspace, lookup.workspace.zone()),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to stop workspace", &result.stderr));
    }

    log::info!("workspace {} stop initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_suspend_workspace(workspace: String) -> Result<(), String> {
    log::info!("suspending workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        suspend_workspace_args(&workspace, lookup.workspace.zone()),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to suspend workspace", &result.stderr));
    }

    log::info!("workspace {} suspend initiated", lookup.workspace.name());
    Ok(())
}

#[tauri::command]
pub async fn workspaces_get_workspace(
    state: State<'_, crate::state::WorkspaceMetadataManager>,
    workspace: String,
) -> Result<Workspace, String> {
    log::trace!("getting workspace {workspace}");
    Ok(state.apply_workspace_state(find_workspace(&workspace).await?.workspace))
}

#[tauri::command]
pub fn workspaces_set_active_session(
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

    let mut entries = active_session_metadata_entries(Some(&active_session));
    if let Some(key) = workspace_last_active_metadata_key(&kind) {
        entries.push(WorkspaceMetadataEntry {
            key: key.to_string(),
            value: Some(current_rfc3339_timestamp()),
        });
    }
    state.enqueue(&workspace, None, entries);

    Ok(())
}

#[tauri::command]
pub async fn workspaces_submit_prompt(
    state: State<'_, terminal::TerminalManager>,
    workspace: String,
    prompt: String,
    model: String,
) -> Result<terminal::TerminalCreateResult, String> {
    log::info!("submitting {model} prompt for workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;
    if !lookup.workspace.ready() {
        return Err(format!("workspace {workspace} is not ready"));
    }

    let prompt = trim_prompt_input(&prompt)?;
    let command = prompt_command_for_model(&model, &prompt)?;
    let attachment_id =
        terminal::start_terminal_command(state.inner(), &workspace, &command).await?;

    Ok(terminal::TerminalCreateResult { attachment_id })
}

#[tauri::command]
pub async fn workspaces_delete_workspace(workspace: String) -> Result<(), String> {
    log::info!("deleting workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    run_gcloud_detached(
        &lookup.account,
        &lookup.gcloud_project,
        delete_workspace_args(&workspace, lookup.workspace.zone()),
    )
    .await?;

    log::info!("workspace {} delete initiated", lookup.workspace.name());
    Ok(())
}

pub(crate) async fn set_workspace_label(
    workspace: &str,
    label: &str,
    value: &str,
) -> Result<(), String> {
    let lookup = find_workspace(workspace).await?;
    update_workspace_label_in_lookup(lookup, label, value).await
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

fn prompt_command_for_model(model: &str, prompt: &str) -> Result<String, String> {
    match model.trim() {
        "codex" => Ok(terminal::codex_prompt_command(prompt)),
        "claude" => Ok(terminal::claude_prompt_command(prompt)),
        other => Err(format!("unsupported prompt model: {other}")),
    }
}

pub(crate) async fn find_workspace(name: &str) -> Result<WorkspaceLookup, String> {
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

async fn update_workspace_label_in_lookup(
    lookup: WorkspaceLookup,
    label: &str,
    value: &str,
) -> Result<(), String> {
    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        update_workspace_label_args(&lookup.workspace, label, value),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error(
            &format!(
                "failed to update {} label for workspace {}",
                label,
                lookup.workspace.name()
            ),
            &result.stderr,
        ));
    }

    log::info!(
        "updated {} label for workspace {}",
        label,
        lookup.workspace.name()
    );
    Ok(())
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

pub(crate) async fn apply_workspace_metadata_entries_in_lookup(
    lookup: WorkspaceLookup,
    entries: &[WorkspaceMetadataEntry],
) -> Result<(), String> {
    if entries.is_empty() {
        return Err("workspace metadata update did not include any values".to_string());
    }

    let removals = entries
        .iter()
        .filter(|entry| {
            entry
                .value
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        })
        .map(|entry| entry.key.as_str())
        .collect::<Vec<_>>();
    let additions = entries
        .iter()
        .filter_map(|entry| {
            let value = entry.value.as_deref()?.trim();
            (!value.is_empty()).then_some((entry.key.as_str(), value))
        })
        .collect::<Vec<_>>();

    if !removals.is_empty() {
        let result = run_gcloud(
            &lookup.account,
            &lookup.gcloud_project,
            remove_workspace_metadata_args(&lookup.workspace, &removals),
        )
        .await?;
        if !result.success {
            return Err(gcloud_error(
                &format!(
                    "failed to remove metadata for workspace {}",
                    lookup.workspace.name()
                ),
                &result.stderr,
            ));
        }
    }

    if !additions.is_empty() {
        let result = run_gcloud(
            &lookup.account,
            &lookup.gcloud_project,
            add_workspace_metadata_args(&lookup.workspace, &additions)?,
        )
        .await?;
        if !result.success {
            return Err(gcloud_error(
                &format!(
                    "failed to update metadata for workspace {}",
                    lookup.workspace.name()
                ),
                &result.stderr,
            ));
        }
    }

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
    let result = run_gcloud(
        account,
        project,
        [
            "compute".to_string(),
            "instances".to_string(),
            "list".to_string(),
            "--format=json(name,zone,status,labels,metadata,creationTimestamp)".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to list workspaces", &result.stderr));
    }

    parse_workspaces(&result.stdout)
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
            ready: false,
        },
        branch_label.to_string(),
        target_branch.to_string(),
        false,
        None,
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
            ready: false,
        },
        unread: false,
        working: None,
        terminals: Vec::new(),
        browsers: Vec::new(),
        template: true,
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
    if !config.projects.contains_key(project) {
        return Err(format!("project not found: {project}"));
    }

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

    let result = run_gcloud(
        &gcloud.account,
        &gcloud.project,
        create_template_workspace_args(&workspace_name, project, boot_source.as_deref(), &gcloud),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error(
            "failed to create template workspace",
            &result.stderr,
        ));
    }

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

fn create_workspace_args(
    workspace_name: &str,
    project_label: &str,
    branch_label: &str,
    target_branch: &str,
    boot_source: &WorkspaceBootSource,
    gcloud: &ResolvedGcloudConfig,
) -> Vec<String> {
    let labels = vec![
        format!("project={}", sanitize_label_value(project_label)),
        "ready=false".to_string(),
    ];
    let metadata =
        workspace_metadata_arg(&[("branch", branch_label), ("target_branch", target_branch)])
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
            "--labels=project={},template=true,ready=false",
            sanitize_label_value(project_label)
        ),
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

fn update_workspace_label_args(workspace: &Workspace, label: &str, value: &str) -> Vec<String> {
    let sanitized_value = sanitize_label_value(value);
    if sanitized_value.is_empty() {
        vec![
            "compute".to_string(),
            "instances".to_string(),
            "remove-labels".to_string(),
            workspace.name().to_string(),
            format!("--zone={}", workspace.zone()),
            format!("--labels={label}"),
        ]
    } else {
        vec![
            "compute".to_string(),
            "instances".to_string(),
            "add-labels".to_string(),
            workspace.name().to_string(),
            format!("--zone={}", workspace.zone()),
            format!("--labels={label}={sanitized_value}"),
        ]
    }
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

pub(crate) async fn run_gcloud<I, S>(
    account: &str,
    project: &str,
    args: I,
) -> Result<CommandResult, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let account = account.to_string();
    let project = project.to_string();
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let command_line = args.join(" ");

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut command = Command::new("gcloud");
        if !account.trim().is_empty() {
            command.arg(format!("--account={account}"));
        }
        if !project.trim().is_empty() {
            command.arg(format!("--project={project}"));
        }
        let output = command
            .args(&args)
            .output()
            .map_err(|error| format!("failed to execute gcloud: {error}"))?;
        if output.status.success() {
            log::trace!(
                "workspace gcloud command completed duration_ms={} project={} args={command_line}",
                started.elapsed().as_millis(),
                project
            );
        } else {
            log::warn!(
                "workspace gcloud command failed duration_ms={} project={} args={} stderr={}",
                started.elapsed().as_millis(),
                project,
                command_line,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Ok(CommandResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    })
    .await
    .map_err(|error| format!("gcloud task failed: {error}"))?
}

async fn run_gcloud_detached<I, S>(account: &str, project: &str, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let account = account.to_string();
    let project = project.to_string();
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let command_line = args.join(" ");

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut command = Command::new("gcloud");
        if !account.trim().is_empty() {
            command.arg(format!("--account={account}"));
        }
        if !project.trim().is_empty() {
            command.arg(format!("--project={project}"));
        }
        command
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|error| format!("failed to execute gcloud: {error}"))?;

        log::trace!(
            "workspace gcloud command detached duration_ms={} project={} args={command_line}",
            started.elapsed().as_millis(),
            project
        );
        Ok(())
    })
    .await
    .map_err(|error| format!("gcloud task failed: {error}"))?
}

pub(crate) async fn stop_and_snapshot_template_workspace(
    account: String,
    gcloud_project: String,
    project: String,
    workspace_name: String,
    zone: String,
) -> Result<(), String> {
    let instance =
        describe_instance_in_project(&workspace_name, &account, &gcloud_project, &zone).await?;
    let instance = if instance.status == INSTANCE_STATUS_TERMINATED {
        instance
    } else {
        let stop_result = run_gcloud(
            &account,
            &gcloud_project,
            stop_workspace_args(&workspace_name, &zone),
        )
        .await?;

        if !stop_result.success {
            return Err(gcloud_error(
                "failed to stop template workspace",
                &stop_result.stderr,
            ));
        }

        wait_for_instance_terminated(&account, &gcloud_project, &workspace_name, &zone).await?
    };
    let snapshot_name = generate_template_snapshot_name(&project);
    let snapshot_result = run_gcloud(
        &account,
        &gcloud_project,
        create_template_snapshot_args(&snapshot_name, &instance.boot_disk, &zone, &project),
    )
    .await?;

    if !snapshot_result.success {
        return Err(gcloud_error(
            "failed to create template snapshot",
            &snapshot_result.stderr,
        ));
    }

    wait_for_snapshot_ready(&account, &gcloud_project, &snapshot_name).await?;

    let snapshots = list_template_snapshots_in_project(&account, &gcloud_project, &project).await?;
    for snapshot in snapshots
        .into_iter()
        .filter(|snapshot| snapshot.name != snapshot_name)
    {
        let delete_result = run_gcloud(
            &account,
            &gcloud_project,
            delete_snapshot_args(&snapshot.name),
        )
        .await?;
        if delete_result.success {
            log::info!(
                "deleted older template snapshot {} for project {}",
                snapshot.name,
                project
            );
        } else {
            log::warn!(
                "failed to delete older template snapshot {} for project {}: {}",
                snapshot.name,
                project,
                delete_result.stderr.trim()
            );
        }
    }

    log::info!(
        "template workspace {} snapshot refresh started with snapshot {}",
        workspace_name,
        snapshot_name
    );
    Ok(())
}

async fn wait_for_instance_terminated(
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

async fn describe_instance_in_project(
    name: &str,
    account: &str,
    project: &str,
    zone: &str,
) -> Result<InstanceState, String> {
    let result = run_gcloud(
        account,
        project,
        [
            "compute".to_string(),
            "instances".to_string(),
            "describe".to_string(),
            name.to_string(),
            format!("--zone={zone}"),
            "--format=json(status,disks)".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to describe workspace", &result.stderr));
    }

    parse_instance_state(&result.stdout)
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

async fn wait_for_snapshot_ready(
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

async fn describe_snapshot_in_project(
    snapshot_name: &str,
    account: &str,
    project: &str,
) -> Result<Snapshot, String> {
    let result = run_gcloud(
        account,
        project,
        [
            "compute".to_string(),
            "snapshots".to_string(),
            "describe".to_string(),
            snapshot_name.to_string(),
            "--format=json(name,status,creationTimestamp,labels)".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error(
            "failed to describe template snapshot",
            &result.stderr,
        ));
    }

    let value: Value = serde_json::from_str(&result.stdout)
        .map_err(|error| format!("invalid gcloud json: {error}"))?;
    parse_snapshot(&value)
}

pub(crate) async fn list_template_snapshots_in_project(
    account: &str,
    project: &str,
    project_label: &str,
) -> Result<Vec<Snapshot>, String> {
    let result = run_gcloud(
        account,
        project,
        [
            "compute".to_string(),
            "snapshots".to_string(),
            "list".to_string(),
            "--format=json(name,status,creationTimestamp,labels)".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error(
            "failed to list template snapshots",
            &result.stderr,
        ));
    }

    let mut snapshots = parse_snapshots(&result.stdout)?
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
    let result = run_gcloud(
        account,
        project,
        [
            "compute".to_string(),
            "snapshots".to_string(),
            "list".to_string(),
            "--format=json(name,status,creationTimestamp,labels)".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error(
            "failed to list template snapshots",
            &result.stderr,
        ));
    }

    Ok(parse_snapshots(&result.stdout)?
        .into_iter()
        .filter(|snapshot| snapshot.template && snapshot.project.is_some())
        .collect())
}

fn parse_workspaces(stdout: &str) -> Result<Vec<Workspace>, String> {
    let value: Value =
        serde_json::from_str(stdout).map_err(|error| format!("invalid gcloud json: {error}"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| "gcloud did not return a JSON array".to_string())?;

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
    let active_session = resolve_workspace_active_session(&metadata);
    let ready = labels
        .get("ready")
        .and_then(Value::as_str)
        .map(|value| parse_bool_value("ready", value))
        .transpose()?
        .unwrap_or(status == "RUNNING");

    let base = WorkspaceBase {
        name,
        project,
        last_active,
        last_working,
        active_session,
        created_at,
        status,
        zone,
        ready,
    };

    let unread =
        parse_optional_bool(&metadata, TERMINAL_UNREAD_METADATA_KEY, "unread")?.unwrap_or(false);
    let working = parse_optional_bool(&metadata, TERMINAL_WORKING_METADATA_KEY, "working")?;
    let terminals = parse_prefixed_workspace_sessions(&metadata, TERMINAL_SESSION_METADATA_PREFIX);
    let browsers = parse_prefixed_workspace_sessions(&metadata, BROWSER_SESSION_METADATA_PREFIX);

    if template {
        Ok(Workspace::Template(TemplateWorkspace {
            base,
            unread,
            working,
            terminals,
            browsers,
            template: true,
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

fn parse_prefixed_workspace_sessions(
    metadata: &HashMap<String, String>,
    prefix: &str,
) -> Vec<WorkspaceSession> {
    let mut sessions = metadata
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(prefix)
                .map(|attachment_id| (attachment_id, value.as_str()))
        })
        .filter_map(|(attachment_id, value)| {
            let mut session = serde_json::from_str::<WorkspaceSession>(value).ok()?;
            if session.attachment_id.trim().is_empty() {
                session.attachment_id = attachment_id.to_string();
            }
            Some(session)
        })
        .filter(|session| {
            !session.kind.trim().is_empty()
                && !session.name.trim().is_empty()
                && !session.attachment_id.trim().is_empty()
        })
        .collect::<Vec<_>>();
    terminal::sort_workspace_sessions_oldest_to_newest(&mut sessions);
    sessions
}

fn resolve_workspace_last_active(metadata: &HashMap<String, String>) -> Option<String> {
    [
        metadata.get(BROWSER_LAST_ACTIVE_METADATA_KEY),
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

fn resolve_workspace_active_session(
    metadata: &HashMap<String, String>,
) -> Option<WorkspaceActiveSession> {
    let kind = metadata
        .get(ACTIVE_SESSION_KIND_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let attachment_id = metadata
        .get(ACTIVE_SESSION_ATTACHMENT_ID_METADATA_KEY)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    Some(WorkspaceActiveSession::new(kind, attachment_id))
}

fn parse_bool_value(label: &str, value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("invalid {label} label value: {other}")),
    }
}

fn current_rfc3339_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn workspace_metadata_arg(entries: &[(&str, &str)]) -> Result<String, String> {
    const DELIMITER_CANDIDATES: &[&str] = &["|", ";", "#", "@@", "SILO_METADATA_DELIM"];

    let populated = entries
        .iter()
        .copied()
        .filter(|(_, value)| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if populated.is_empty() {
        return Err("workspace metadata update did not include any values".to_string());
    }

    let delimiter = DELIMITER_CANDIDATES
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

fn generate_template_snapshot_name(project: &str) -> String {
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

pub(crate) fn gcloud_error(context: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {stderr}")
    }
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
            ready: true,
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
        assert!(args.contains(&"--labels=project=demo-project,ready=false".to_string()));
        assert!(args.contains(&"--metadata=^|^branch=Aare|target_branch=Feature/Inbox".to_string()));
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
        assert!(args.contains(&"--labels=project=demo-project,ready=false".to_string()));
        assert!(args.contains(&"--metadata=^|^branch=Aare".to_string()));
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
        assert!(args.contains(&"--labels=project=demo-project,ready=false".to_string()));
        assert!(args.contains(&"--metadata=^|^branch=Aare".to_string()));
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
        assert!(args.contains(&"--labels=project=demo,template=true,ready=false".to_string()));
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
                    { "key": "active-session-kind", "value": "terminal" },
                    { "key": "active-session-attachment-id", "value": "terminal-1" },
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
        assert_eq!(
            workspace.base.active_session,
            Some(WorkspaceActiveSession::new("terminal", "terminal-1"))
        );
        assert_eq!(workspace.terminals.len(), 1);
        assert_eq!(workspace.terminals[0].name, "codex");
        assert_eq!(workspace.terminals[0].attachment_id, "terminal-1");
        assert_eq!(workspace.terminals[0].working, Some(true));
        assert_eq!(workspace.terminals[0].unread, Some(false));
    }

    #[test]
    fn parse_workspace_reads_flat_session_metadata() {
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
        assert_eq!(workspace.terminals.len(), 1);
        assert_eq!(workspace.terminals[0].kind, "terminal");
        assert_eq!(workspace.terminals[0].name, "codex");
        assert_eq!(workspace.terminals[0].attachment_id, "terminal-1");
        assert_eq!(workspace.terminals[0].working, Some(true));
        assert_eq!(workspace.terminals[0].unread, Some(false));
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
        assert_eq!(workspace.terminals.len(), 1);
        assert_eq!(workspace.terminals[0].name, "good");
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
                "template": "true",
                "ready": "false"
            }
        }))
        .expect("workspace should parse");

        let Workspace::Template(workspace) = workspace else {
            panic!("workspace should parse as a template workspace");
        };

        assert!(workspace.template);
        assert!(!workspace.base.ready);
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
                "template": "true",
                "ready": "true"
            },
            "metadata": {
                "items": [
                    { "key": "terminal-last-active", "value": "2026-03-16T04:18:39Z" },
                    { "key": "terminal-working", "value": "false" },
                    { "key": "terminal-unread", "value": "false" },
                    { "key": "terminal-session-terminal-1773634611341", "value": "{\"type\":\"terminal\",\"name\":\"bun run build:render\",\"attachment_id\":\"terminal-1773634611341\"}" }
                ]
            }
        }))
        .expect("workspace should parse");

        let Workspace::Template(workspace) = workspace else {
            panic!("workspace should parse as a template workspace");
        };

        assert!(workspace.template);
        assert!(workspace.base.ready);
        assert_eq!(
            workspace.base.last_active.as_deref(),
            Some("2026-03-16T04:18:39Z")
        );
        assert!(!workspace.unread);
        assert_eq!(workspace.working, Some(false));
        assert_eq!(workspace.terminals.len(), 1);
        assert_eq!(workspace.terminals[0].name, "bun run build:render");
        assert_eq!(
            workspace.terminals[0].attachment_id,
            "terminal-1773634611341"
        );
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
        };
        let non_matching = Snapshot {
            name: "other-silo-template-1710000000-123".to_string(),
            status: "READY".to_string(),
            created_at: "2026-03-12T12:00:00Z".to_string(),
            project: Some("other".to_string()),
            template: true,
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
    fn prompt_command_for_model_rejects_unknown_model() {
        assert_eq!(
            prompt_command_for_model("gpt", "ship it").expect_err("unknown model should fail"),
            "unsupported prompt model: gpt"
        );
    }

    #[test]
    fn prompt_command_for_model_uses_claude_command_builder() {
        let command = prompt_command_for_model("claude", "ship it")
            .expect("claude model should build a command");

        assert!(command.starts_with("silo claude "));
        assert!(command.contains("base64 --decode"));
    }
}
