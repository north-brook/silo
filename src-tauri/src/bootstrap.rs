use crate::build_info;
use crate::codex::{codex_token_from_auth_json, normalize_codex_auth_json};
use crate::config::{ConfigStore, ProjectConfig, SiloConfig};
use crate::gcp;
use crate::remote::{
    remote_command_error, run_remote_command, run_remote_command_with_stdin,
    run_terminal_user_command, shell_quote, workspace_shell_command,
    workspace_shell_command_with_credentials, REMOTE_WORKSPACE_AGENT_BIN, TERMINAL_WORKSPACE_DIR,
};
use crate::state::WorkspaceMetadataManager;
use crate::state_paths;
use crate::terminal;
use crate::workspaces::{self, Workspace, WorkspaceLookup};
use crate::AppRuntime;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    LazyLock, Mutex, OnceLock,
};
use std::time::Instant;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const REMOTE_BOOTSTRAP_STATE_FILE: &str = "/home/silo/.silo/workspace-bootstrap-state";
const REMOTE_BOOTSTRAP_LOCK_DIR: &str = "/home/silo/.silo/workspace-bootstrap.lock";
const REMOTE_BOOTSTRAP_LOG_FILE: &str = "/home/silo/.silo/bootstrap.log";
const REMOTE_CREDENTIALS_FILE: &str = "/home/silo/.silo/credentials.sh";
const REMOTE_WORKSPACE_AGENT_PIDFILE: &str = "/home/silo/.silo/workspace-agent/daemon.pid";
pub(crate) const REMOTE_WORKSPACE_AGENT_FINGERPRINT_FILE: &str =
    "/home/silo/.silo/workspace-agent/fingerprint";
const REMOTE_WORKSPACE_AGENT_SHELL_FILE: &str = "/home/silo/.silo/workspace-agent-shell.sh";
const WORKSPACE_AGENT_RELEASE_ROLLOUT_STATE_FILE_NAME: &str =
    "workspace-agent-release-rollout.json";
const WORKSPACE_BOOTSTRAP_VERSION: &str = "21";
const STARTUP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const INSTANCE_RUNNING_POLL_ATTEMPTS: usize = 180;
const SSH_READY_POLL_ATTEMPTS: usize = 120;
const BOOTSTRAP_RETRY_ATTEMPTS: usize = 60;
const OBSERVER_READY_POLL_ATTEMPTS: usize = 30;
const OBSERVER_HEARTBEAT_STALE_AFTER_SECS: i64 = 45;
const WORKSPACE_AGENT_UPDATE_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(15);
const WORKSPACE_AGENT_BIN_BYTES: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/workspace-agent-x86_64-unknown-linux-musl"
));
static WORKSPACE_STARTUP_RECONCILE_STATE: LazyLock<Mutex<HashMap<String, WorkspaceStartupState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static WORKSPACE_AGENT_UPDATE_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static WORKSPACE_AGENT_UPDATE_RETRY_AT: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static WORKSPACE_AGENT_UPDATE_ATTEMPT_IDS: AtomicU64 = AtomicU64::new(1_000_000);
static WORKSPACE_METADATA_MANAGER: OnceLock<WorkspaceMetadataManager> = OnceLock::new();

#[derive(Debug, Clone)]
struct WorkspaceBootstrap {
    remote_url: String,
    target_branch: String,
    workspace_branch: Option<String>,
    gh_username: String,
    gh_token: String,
    codex_auth_json: String,
    codex_auth_fingerprint: String,
    codex_model: String,
    codex_model_reasoning_effort: String,
    claude_token: String,
    claude_model: String,
    claude_effort_level: String,
    claude_always_thinking_enabled: bool,
    git_user_name: String,
    git_user_email: String,
    env_files: Vec<BootstrapEnvFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BootstrapEnvFile {
    pub(crate) relative_path: String,
    pub(crate) contents_base64: String,
    pub(crate) contents_sha256: String,
}

#[derive(Debug, Clone, Default)]
struct WorkspaceStartupState {
    in_flight: bool,
    next_attempt_id: u64,
    bootstrap: Option<WorkspaceBootstrap>,
}

#[derive(Default)]
struct LifecycleReporter {
    attempt_id: u64,
    phase: Option<String>,
    detail: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceAgentReleaseRolloutState {
    app_version: String,
    fingerprint: String,
    completed_at: String,
}

fn begin_workspace_startup_reconcile(workspace: &str) -> Option<u64> {
    let Ok(mut states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() else {
        return None;
    };
    let state = states.entry(workspace.to_string()).or_default();
    if state.in_flight {
        return None;
    }
    state.in_flight = true;
    state.next_attempt_id = state.next_attempt_id.saturating_add(1).max(1);
    Some(state.next_attempt_id)
}

fn finish_workspace_startup_reconcile(workspace: &str, attempt_id: u64) {
    let Ok(mut states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() else {
        return;
    };
    let Some(state) = states.get_mut(workspace) else {
        return;
    };
    if state.next_attempt_id == attempt_id {
        state.in_flight = false;
        state.bootstrap = None;
    }
}

fn cached_workspace_bootstrap(workspace: &str) -> Option<WorkspaceBootstrap> {
    let Ok(states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() else {
        return None;
    };
    states
        .get(workspace)
        .and_then(|state| state.bootstrap.clone())
}

fn cached_workspace_bootstrap_with_fresh_assistant_config(
    workspace: &str,
) -> Option<WorkspaceBootstrap> {
    let mut bootstrap = cached_workspace_bootstrap(workspace)?;
    let Ok(config) = ConfigStore::new().and_then(|store| store.load()) else {
        log::warn!(
            "using cached workspace bootstrap for {} without refreshing assistant config because config reload failed",
            workspace
        );
        return Some(bootstrap);
    };

    let (codex_auth_json, codex_auth_fingerprint) = workspace_bootstrap_codex_auth(&config);
    let codex_model = config.codex.model.clone();
    let codex_model_reasoning_effort = config.codex.model_reasoning_effort.clone();
    let claude_model = config.claude.model.clone();
    let claude_effort_level = config.claude.effort_level.clone();
    let claude_always_thinking_enabled = config.claude.always_thinking_enabled;
    if bootstrap.codex_auth_json == codex_auth_json
        && bootstrap.codex_auth_fingerprint == codex_auth_fingerprint
        && bootstrap.codex_model == codex_model
        && bootstrap.codex_model_reasoning_effort == codex_model_reasoning_effort
        && bootstrap.claude_model == claude_model
        && bootstrap.claude_effort_level == claude_effort_level
        && bootstrap.claude_always_thinking_enabled == claude_always_thinking_enabled
    {
        return Some(bootstrap);
    }

    bootstrap.codex_auth_json = codex_auth_json.clone();
    bootstrap.codex_auth_fingerprint = codex_auth_fingerprint.clone();
    bootstrap.codex_model = codex_model.clone();
    bootstrap.codex_model_reasoning_effort = codex_model_reasoning_effort.clone();
    bootstrap.claude_model = claude_model.clone();
    bootstrap.claude_effort_level = claude_effort_level.clone();
    bootstrap.claude_always_thinking_enabled = claude_always_thinking_enabled;

    if let Ok(mut states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() {
        if let Some(state) = states.get_mut(workspace) {
            if let Some(cached) = state.bootstrap.as_mut() {
                cached.codex_auth_json = codex_auth_json;
                cached.codex_auth_fingerprint = codex_auth_fingerprint;
                cached.codex_model = codex_model;
                cached.codex_model_reasoning_effort = codex_model_reasoning_effort;
                cached.claude_model = claude_model;
                cached.claude_effort_level = claude_effort_level;
                cached.claude_always_thinking_enabled = claude_always_thinking_enabled;
            }
        }
    }

    Some(bootstrap)
}

pub(crate) fn workspace_startup_attempt_in_flight(workspace: &str, attempt_id: u64) -> bool {
    let Ok(states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() else {
        return false;
    };
    states
        .get(workspace)
        .is_some_and(|state| state.in_flight && state.next_attempt_id == attempt_id)
}

fn build_workspace_bootstrap(
    config: &SiloConfig,
    project_name: &str,
    project: &ProjectConfig,
    target_branch: &str,
    workspace_branch: Option<&str>,
) -> Result<WorkspaceBootstrap, String> {
    if project.remote_url.trim().is_empty() {
        return Err(format!("project {project_name} is missing remote_url"));
    }

    let target_branch = target_branch.trim().to_string();
    if target_branch.is_empty() {
        return Err(format!("project {project_name} is missing a target branch"));
    }

    let workspace_branch = match workspace_branch.map(str::trim) {
        Some(branch) if branch.is_empty() => {
            return Err(format!(
                "workspace bootstrap for project {project_name} is missing branch metadata"
            ));
        }
        Some(branch) => Some(branch.to_string()),
        None => None,
    };

    let (codex_auth_json, codex_auth_fingerprint) = workspace_bootstrap_codex_auth(config);

    Ok(WorkspaceBootstrap {
        remote_url: project.remote_url.clone(),
        target_branch,
        workspace_branch,
        gh_username: config.git.gh_username.clone(),
        gh_token: config.git.gh_token.clone(),
        codex_auth_json,
        codex_auth_fingerprint,
        codex_model: config.codex.model.clone(),
        codex_model_reasoning_effort: config.codex.model_reasoning_effort.clone(),
        claude_token: config.claude.token.clone(),
        claude_model: config.claude.model.clone(),
        claude_effort_level: config.claude.effort_level.clone(),
        claude_always_thinking_enabled: config.claude.always_thinking_enabled,
        git_user_name: config.git.user_name.clone(),
        git_user_email: config.git.user_email.clone(),
        env_files: load_bootstrap_env_files(project_name, project),
    })
}

fn workspace_bootstrap_codex_auth(config: &SiloConfig) -> (String, String) {
    let codex_auth_json = normalize_codex_auth_json(&config.codex.auth_json).unwrap_or_default();
    let codex_auth_fingerprint = codex_auth_fingerprint(&codex_auth_json);
    (codex_auth_json, codex_auth_fingerprint)
}

async fn bootstrap_workspace(lookup: &WorkspaceLookup) -> Result<(), String> {
    let started = Instant::now();
    log::info!("bootstrapping workspace {}", lookup.workspace.name());
    let bootstrap = workspace_bootstrap(lookup)?;
    let script = workspace_bootstrap_script(lookup, &bootstrap);
    let result = run_remote_command_with_stdin(
        lookup,
        &run_terminal_user_command("bash -se"),
        script.into_bytes(),
    )
    .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to bootstrap workspace",
            &result.stderr,
        ));
    }
    log::info!(
        "workspace {} bootstrap completed duration_ms={}",
        lookup.workspace.name(),
        started.elapsed().as_millis()
    );

    Ok(())
}

pub(crate) fn start_template_bootstrap(workspace: String) {
    start_workspace_startup_reconcile(workspace);
}

pub(crate) fn initialize_workspace_metadata_manager(manager: WorkspaceMetadataManager) {
    let _ = WORKSPACE_METADATA_MANAGER.set(manager);
}

pub(crate) fn cache_workspace_bootstrap(
    workspace: &str,
    config: &SiloConfig,
    project_name: &str,
    project: &ProjectConfig,
    target_branch: &str,
    workspace_branch: Option<&str>,
) -> Result<(), String> {
    let bootstrap = build_workspace_bootstrap(
        config,
        project_name,
        project,
        target_branch,
        workspace_branch,
    )?;
    let Ok(mut states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() else {
        return Ok(());
    };
    states.entry(workspace.to_string()).or_default().bootstrap = Some(bootstrap);
    Ok(())
}

fn workspace_metadata_manager() -> Option<&'static WorkspaceMetadataManager> {
    WORKSPACE_METADATA_MANAGER.get()
}

fn next_workspace_agent_update_attempt_id() -> u64 {
    WORKSPACE_AGENT_UPDATE_ATTEMPT_IDS.fetch_add(1, Ordering::Relaxed)
}

fn begin_workspace_agent_update_reconcile(workspace: &str) -> Option<u64> {
    let now = Instant::now();
    if let Ok(mut retry_at) = WORKSPACE_AGENT_UPDATE_RETRY_AT.lock() {
        if let Some(deadline) = retry_at.get(workspace).copied() {
            if now < deadline {
                return None;
            }
        }
        retry_at.remove(workspace);
    }

    let Ok(mut in_flight) = WORKSPACE_AGENT_UPDATE_IN_FLIGHT.lock() else {
        return None;
    };
    if !in_flight.insert(workspace.to_string()) {
        return None;
    }

    Some(next_workspace_agent_update_attempt_id())
}

fn finish_workspace_agent_update_reconcile(workspace: &str, success: bool) {
    if let Ok(mut in_flight) = WORKSPACE_AGENT_UPDATE_IN_FLIGHT.lock() {
        in_flight.remove(workspace);
    }
    if let Ok(mut retry_at) = WORKSPACE_AGENT_UPDATE_RETRY_AT.lock() {
        if success {
            retry_at.remove(workspace);
        } else {
            retry_at.insert(
                workspace.to_string(),
                Instant::now() + WORKSPACE_AGENT_UPDATE_RETRY_AFTER,
            );
        }
    }
}

pub(crate) fn workspace_agent_fingerprint() -> String {
    workspace_agent_source_signature()
}

fn workspace_agent_release_rollout_state_path() -> Result<std::path::PathBuf, String> {
    Ok(state_paths::app_state_dir()?.join(WORKSPACE_AGENT_RELEASE_ROLLOUT_STATE_FILE_NAME))
}

fn load_workspace_agent_release_rollout_state(
) -> Result<Option<WorkspaceAgentReleaseRolloutState>, String> {
    let path = workspace_agent_release_rollout_state_path()?;
    let Ok(contents) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn save_workspace_agent_release_rollout_state(
    state: &WorkspaceAgentReleaseRolloutState,
) -> Result<(), String> {
    let path = workspace_agent_release_rollout_state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create workspace agent rollout state directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let contents = serde_json::to_string_pretty(state)
        .map_err(|error| format!("failed to encode rollout state: {error}"))?;
    fs::write(&path, contents)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub(crate) fn start_workspace_startup_reconcile_if_needed(workspace: Workspace) {
    if workspace.should_reconcile_startup() {
        if !workspace_has_local_bootstrap_context(&workspace) {
            log::debug!(
                "skipping startup reconcile for workspace {} because the project is not configured locally",
                workspace.name()
            );
            return;
        }
        start_workspace_startup_reconcile(workspace.name().to_string());
    }
}

fn workspace_has_local_bootstrap_context(workspace: &Workspace) -> bool {
    if cached_workspace_bootstrap(workspace.name()).is_some() {
        return true;
    }

    let Some(project_name) = workspace.project() else {
        return false;
    };
    let Ok(config) = ConfigStore::new().and_then(|store| store.load()) else {
        return false;
    };

    config.projects.contains_key(project_name)
}

pub(crate) fn start_workspace_startup_reconcile(workspace: String) {
    let Some(attempt_id) = begin_workspace_startup_reconcile(&workspace) else {
        return;
    };

    tauri::async_runtime::spawn(async move {
        let result = reconcile_workspace_startup(&workspace, attempt_id).await;
        if let Err(error) = result {
            if let Some(manager) = workspace_metadata_manager() {
                manager.enqueue_workspace_lifecycle(
                    &workspace,
                    None,
                    attempt_id,
                    "failed",
                    Some("Workspace startup failed"),
                    Some(&error),
                );
            } else {
                log::warn!(
                    "workspace metadata manager unavailable while publishing startup failure for {}",
                    workspace
                );
            }
            log::warn!(
                "background workspace startup reconcile failed for workspace {}: {}",
                workspace,
                error
            );
        } else {
            log::info!(
                "background workspace startup reconcile completed for workspace {}",
                workspace
            );
        }

        finish_workspace_startup_reconcile(&workspace, attempt_id);
    });
}

fn workspace_agent_update_allowed_phase(phase: &str) -> bool {
    matches!(phase, "ready" | "updating_workspace_agent")
}

fn workspace_needs_agent_update(workspace: &Workspace) -> bool {
    if workspace.status() != "RUNNING" {
        return false;
    }
    if !workspace_agent_update_allowed_phase(workspace.lifecycle().phase()) {
        return false;
    }

    let expected = workspace_agent_fingerprint();
    workspace.agent_fingerprint() != Some(expected.as_str())
}

fn workspace_has_stale_agent_update_state(workspace: &Workspace) -> bool {
    let expected = workspace_agent_fingerprint();
    workspace_has_stale_agent_update_state_with_expected(
        workspace.status(),
        workspace.lifecycle().phase(),
        workspace.agent_fingerprint(),
        expected.as_str(),
    )
}

fn workspace_has_stale_agent_update_state_with_expected(
    status: &str,
    phase: &str,
    agent_fingerprint: Option<&str>,
    expected_fingerprint: &str,
) -> bool {
    status == "RUNNING"
        && phase == "updating_workspace_agent"
        && agent_fingerprint == Some(expected_fingerprint)
}

fn workspace_agent_update_reconcile_needed(workspace: &Workspace) -> bool {
    workspace_needs_agent_update(workspace) || workspace_has_stale_agent_update_state(workspace)
}

pub(crate) fn start_release_workspace_agent_rollout_if_needed(
    app_handle: tauri::AppHandle<AppRuntime>,
) {
    if !build_info::is_production_build() {
        return;
    }
    if !gcp::runtime_identity_configured() {
        log::info!(
            "skipping workspace agent release rollout because runtime gcp identity is unavailable"
        );
        return;
    }

    let app_version = app_handle.package_info().version.to_string();
    let fingerprint = workspace_agent_fingerprint();
    match load_workspace_agent_release_rollout_state() {
        Ok(Some(state)) if state.app_version == app_version && state.fingerprint == fingerprint => {
            return;
        }
        Ok(_) => {}
        Err(error) => {
            log::warn!("failed to load workspace agent rollout state: {error}");
        }
    }

    tauri::async_runtime::spawn(async move {
        match workspaces::list_all_workspaces().await {
            Ok(workspaces) => {
                let mut pending_updates = 0usize;
                for workspace in workspaces {
                    if workspace_agent_update_reconcile_needed(&workspace) {
                        pending_updates += 1;
                    }
                    start_workspace_agent_update_reconcile_if_needed(workspace);
                }
                if pending_updates == 0 {
                    let state = WorkspaceAgentReleaseRolloutState {
                        app_version,
                        fingerprint,
                        completed_at: workspaces::current_rfc3339_timestamp(),
                    };
                    if let Err(error) = save_workspace_agent_release_rollout_state(&state) {
                        log::warn!("failed to save workspace agent rollout state: {error}");
                    }
                } else {
                    log::info!(
                        "workspace agent release rollout queued updates for {} running workspaces",
                        pending_updates
                    );
                }
            }
            Err(error) => {
                log::warn!(
                    "failed to list workspaces for workspace agent release rollout: {error}"
                );
            }
        }
    });
}

pub(crate) fn start_workspace_agent_update_reconcile_if_needed(workspace: Workspace) {
    if !build_info::is_production_build() || !gcp::runtime_identity_configured() {
        return;
    }
    if !workspace_agent_update_reconcile_needed(&workspace) {
        return;
    }

    let workspace_name = workspace.name().to_string();
    let Some(attempt_id) = begin_workspace_agent_update_reconcile(&workspace_name) else {
        return;
    };

    if let Some(manager) = workspace_metadata_manager() {
        manager.enqueue_workspace_lifecycle(
            &workspace_name,
            None,
            attempt_id,
            "updating_workspace_agent",
            Some("Updating workspace observer"),
            None,
        );
    }

    tauri::async_runtime::spawn(async move {
        let result = reconcile_workspace_agent_update(&workspace_name, attempt_id).await;
        match result {
            Ok(()) => finish_workspace_agent_update_reconcile(&workspace_name, true),
            Err(error) => {
                if let Some(manager) = workspace_metadata_manager() {
                    manager.enqueue_workspace_lifecycle(
                        &workspace_name,
                        None,
                        attempt_id,
                        "updating_workspace_agent",
                        Some("Updating workspace observer"),
                        Some(&error),
                    );
                }
                log::warn!(
                    "workspace agent update reconcile failed for workspace {}: {}",
                    workspace_name,
                    error
                );
                finish_workspace_agent_update_reconcile(&workspace_name, false);
            }
        }
    });
}

async fn reconcile_workspace_agent_update(workspace: &str, attempt_id: u64) -> Result<(), String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if lookup.workspace.status() != "RUNNING" {
        return Ok(());
    }
    if !workspace_agent_update_allowed_phase(lookup.workspace.lifecycle().phase()) {
        return Ok(());
    }

    let expected_fingerprint = workspace_agent_fingerprint();
    if lookup.workspace.agent_fingerprint() == Some(expected_fingerprint.as_str()) {
        if lookup.workspace.lifecycle().phase() == "updating_workspace_agent" {
            wait_for_workspace_agent(workspace, Some(expected_fingerprint.as_str())).await?;
        }
        if let Some(manager) = workspace_metadata_manager() {
            manager.enqueue_workspace_lifecycle(
                workspace,
                Some(lookup),
                attempt_id,
                "ready",
                None,
                None,
            );
        }
        return Ok(());
    }

    if let Some(manager) = workspace_metadata_manager() {
        manager.enqueue_workspace_lifecycle(
            workspace,
            Some(lookup.clone()),
            attempt_id,
            "updating_workspace_agent",
            Some("Updating workspace observer"),
            None,
        );
    }

    wait_for_workspace_ssh(&lookup).await?;

    let bootstrap = match workspace_bootstrap(&lookup) {
        Ok(bootstrap) => Some(bootstrap),
        Err(error) => {
            log::warn!(
                "skipping assistant config sync during workspace agent update for {}: {}",
                lookup.workspace.name(),
                error
            );
            None
        }
    };
    let install_script = workspace_agent_update_script(&lookup, bootstrap.as_ref());
    let install_result = run_remote_command_with_stdin(
        &lookup,
        &run_terminal_user_command("bash -se"),
        install_script.into_bytes(),
    )
    .await?;
    if !install_result.success {
        return Err(remote_command_error(
            "failed to update workspace agent",
            &install_result.stderr,
        ));
    }

    wait_for_workspace_agent(workspace, Some(expected_fingerprint.as_str())).await?;

    if let Some(manager) = workspace_metadata_manager() {
        manager.enqueue_workspace_lifecycle(workspace, None, attempt_id, "ready", None, None);
    }

    Ok(())
}

pub(crate) async fn wait_for_template_bootstrap(workspace: &str) -> Result<(), String> {
    for _ in 0..INSTANCE_RUNNING_POLL_ATTEMPTS {
        let lookup = workspaces::find_workspace(workspace).await?;
        if !lookup.workspace.is_template() {
            return Err(format!("workspace {workspace} is not a template workspace"));
        }

        if lookup.workspace.is_ready() {
            return Ok(());
        }

        start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());

        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "template workspace {workspace} did not finish bootstrapping in time"
    ))
}

pub(crate) async fn clear_template_runtime_state(workspace: &str) -> Result<(), String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if !lookup.workspace.is_template() {
        return Err(format!("workspace {workspace} is not a template workspace"));
    }

    let command = clear_template_runtime_state_command();
    let result =
        run_remote_command(&lookup, &workspace_shell_command_with_credentials(&command)).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to clear template runtime state",
            &result.stderr,
        ));
    }

    Ok(())
}

pub(crate) fn should_retry_template_bootstrap(error: &str) -> bool {
    if is_retryable_terminal_transport_error(error) {
        return true;
    }

    let lower = error.to_ascii_lowercase();
    ["system is booting up", "not permitted to log in yet"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub(crate) fn is_retryable_terminal_transport_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "can't assign requested address",
        "broken pipe",
        "connection refused",
        "connection reset",
        "connection reset by peer",
        "connection timed out",
        "connection closed",
        "connection lost",
        "network is unreachable",
        "operation timed out",
        "port 22",
        "software caused connection abort",
        "timed out",
        "transport endpoint is not connected",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn reconcile_workspace_startup(workspace: &str, attempt_id: u64) -> Result<(), String> {
    let started = Instant::now();
    let mut reporter = LifecycleReporter {
        attempt_id,
        ..LifecycleReporter::default()
    };
    let lookup = wait_for_workspace_running(workspace).await?;

    reporter
        .publish(
            &lookup,
            "waiting_for_ssh",
            Some("Waiting for the VM to accept SSH connections"),
            None,
        )
        .await?;
    wait_for_workspace_ssh(&lookup).await?;

    reporter
        .publish(
            &lookup,
            "bootstrapping",
            Some("Preparing repository, credentials, and tools"),
            None,
        )
        .await?;
    bootstrap_workspace_until_ready(&lookup).await?;

    reporter
        .publish(
            &lookup,
            "waiting_for_agent",
            Some("Waiting for workspace services to come online"),
            None,
        )
        .await?;
    ensure_workspace_agent_running(&lookup).await?;
    let expected_fingerprint = workspace_agent_fingerprint();
    wait_for_workspace_agent(workspace, Some(expected_fingerprint.as_str())).await?;

    if lookup.workspace.is_template() {
        reporter
            .publish(
                &lookup,
                "starting_terminal",
                Some("Opening the default terminal session"),
                None,
            )
            .await?;
        let manager = workspace_metadata_manager().ok_or_else(|| {
            "workspace metadata manager unavailable while opening template terminal".to_string()
        })?;
        let attachment_id =
            terminal::ensure_template_startup_terminal_session(manager, workspace).await?;
        log::info!(
            "template workspace {} prepared default terminal session {}",
            workspace,
            attachment_id
        );
    }

    reporter.publish(&lookup, "ready", None, None).await?;
    log::info!(
        "workspace {} startup lifecycle reached ready duration_ms={}",
        workspace,
        started.elapsed().as_millis()
    );
    Ok(())
}

async fn wait_for_workspace_running(workspace: &str) -> Result<WorkspaceLookup, String> {
    for attempt in 0..INSTANCE_RUNNING_POLL_ATTEMPTS {
        let lookup = match workspaces::find_workspace(workspace).await {
            Ok(lookup) => lookup,
            Err(error) => {
                if attempt + 1 == INSTANCE_RUNNING_POLL_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(STARTUP_POLL_INTERVAL);
                continue;
            }
        };
        if lookup.workspace.status() == "RUNNING" {
            return Ok(lookup);
        }
        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace {workspace} did not reach RUNNING after {} seconds",
        INSTANCE_RUNNING_POLL_ATTEMPTS * STARTUP_POLL_INTERVAL.as_secs() as usize
    ))
}

async fn wait_for_workspace_ssh(lookup: &WorkspaceLookup) -> Result<(), String> {
    let started = Instant::now();
    for attempt in 0..SSH_READY_POLL_ATTEMPTS {
        let result = run_remote_command(lookup, &run_terminal_user_command("true")).await;
        match result {
            Ok(result) if result.success => {
                log::info!(
                    "workspace {} ssh probe succeeded attempt={} elapsed_ms={}",
                    lookup.workspace.name(),
                    attempt + 1,
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            Ok(result) => {
                let error = result.stderr.trim().to_string();
                if attempt + 1 == SSH_READY_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(remote_command_error(
                        "failed to verify workspace ssh readiness",
                        &result.stderr,
                    ));
                }
            }
            Err(error) => {
                if attempt + 1 == SSH_READY_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(error);
                }
            }
        }

        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace {} did not become ssh-ready after {} seconds",
        lookup.workspace.name(),
        SSH_READY_POLL_ATTEMPTS * STARTUP_POLL_INTERVAL.as_secs() as usize
    ))
}

async fn bootstrap_workspace_until_ready(lookup: &WorkspaceLookup) -> Result<(), String> {
    for attempt in 0..BOOTSTRAP_RETRY_ATTEMPTS {
        match bootstrap_workspace(lookup).await {
            Ok(()) => return Ok(()),
            Err(error) if should_retry_template_bootstrap(&error) => {
                log::warn!(
                    "workspace {} bootstrap attempt {}/{} failed with retryable error: {}",
                    lookup.workspace.name(),
                    attempt + 1,
                    BOOTSTRAP_RETRY_ATTEMPTS,
                    error
                );
                if attempt + 1 == BOOTSTRAP_RETRY_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(STARTUP_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }

    Err(format!(
        "workspace {} did not finish bootstrapping in time",
        lookup.workspace.name()
    ))
}

async fn ensure_workspace_agent_running(lookup: &WorkspaceLookup) -> Result<(), String> {
    let check_command = workspace_agent_running_check_remote_command();
    let check_result = run_remote_command(lookup, &check_command).await?;
    if check_result.success {
        return Ok(());
    }

    let install_script = workspace_agent_install_script(lookup);
    let install_result = run_remote_command_with_stdin(
        lookup,
        &run_terminal_user_command("bash -se"),
        install_script.into_bytes(),
    )
    .await?;
    if !install_result.success {
        return Err(remote_command_error(
            "failed to start workspace agent",
            &install_result.stderr,
        ));
    }

    Ok(())
}

async fn wait_for_workspace_agent(
    workspace: &str,
    expected_fingerprint: Option<&str>,
) -> Result<(), String> {
    let ready_check_command = workspace_agent_ready_check_remote_command();
    let mut saw_fresh_heartbeat = false;
    let mut matched_expected_fingerprint = expected_fingerprint.is_none();
    let mut last_ready_probe_error = None::<String>;
    for attempt in 0..OBSERVER_READY_POLL_ATTEMPTS {
        let lookup = workspaces::find_workspace(workspace).await?;
        let heartbeat_is_fresh = agent_heartbeat_is_fresh(&lookup.workspace);
        let fingerprint_matches = expected_fingerprint
            .is_none_or(|expected| lookup.workspace.agent_fingerprint() == Some(expected));
        saw_fresh_heartbeat |= heartbeat_is_fresh;
        matched_expected_fingerprint |= fingerprint_matches;

        if heartbeat_is_fresh && fingerprint_matches {
            match run_remote_command(&lookup, &ready_check_command).await {
                Ok(result) if result.success => return Ok(()),
                Ok(result) => {
                    last_ready_probe_error = Some(remote_command_error(
                        "workspace agent readiness probe failed",
                        &result.stderr,
                    ));
                }
                Err(error) => last_ready_probe_error = Some(error),
            }
        }
        if attempt + 1 == OBSERVER_READY_POLL_ATTEMPTS {
            break;
        }
        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(workspace_agent_wait_timeout_error(
        workspace,
        expected_fingerprint,
        saw_fresh_heartbeat,
        matched_expected_fingerprint,
        last_ready_probe_error.as_deref(),
    ))
}

fn agent_heartbeat_is_fresh(workspace: &Workspace) -> bool {
    let Some(heartbeat) = workspace.agent_heartbeat_at() else {
        return false;
    };
    let Ok(heartbeat) = OffsetDateTime::parse(heartbeat, &Rfc3339) else {
        return false;
    };

    OffsetDateTime::now_utc() - heartbeat
        <= time::Duration::seconds(OBSERVER_HEARTBEAT_STALE_AFTER_SECS)
}

fn workspace_agent_wait_timeout_error(
    workspace: &str,
    expected_fingerprint: Option<&str>,
    saw_fresh_heartbeat: bool,
    matched_expected_fingerprint: bool,
    last_ready_probe_error: Option<&str>,
) -> String {
    if let Some(error) = last_ready_probe_error {
        return format!(
            "workspace agent for {workspace} did not accept the readiness probe: {error}"
        );
    }
    if !saw_fresh_heartbeat {
        return format!("workspace agent for {workspace} did not publish a recent heartbeat");
    }
    if expected_fingerprint.is_some() && !matched_expected_fingerprint {
        return format!("workspace agent for {workspace} did not publish the expected fingerprint");
    }
    if expected_fingerprint.is_some() {
        return format!(
            "workspace agent for {workspace} did not become ready after publishing the expected fingerprint"
        );
    }

    format!(
        "workspace agent for {workspace} did not become ready after publishing a recent heartbeat"
    )
}

impl LifecycleReporter {
    async fn publish(
        &mut self,
        lookup: &WorkspaceLookup,
        phase: &str,
        detail: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<(), String> {
        let next_phase = Some(phase.to_string());
        let next_detail = detail.map(|value| value.to_string());
        let next_error = last_error.map(|value| value.to_string());
        if self.phase == next_phase && self.detail == next_detail && self.last_error == next_error {
            return Ok(());
        }

        if let Some(manager) = workspace_metadata_manager() {
            manager.enqueue_workspace_lifecycle(
                lookup.workspace.name(),
                Some(lookup.clone()),
                self.attempt_id,
                phase,
                detail,
                last_error,
            );
        } else {
            log::warn!(
                "workspace metadata manager unavailable while publishing lifecycle for {}",
                lookup.workspace.name()
            );
        }
        self.phase = next_phase;
        self.detail = next_detail;
        self.last_error = next_error;
        Ok(())
    }
}

fn workspace_bootstrap(lookup: &WorkspaceLookup) -> Result<WorkspaceBootstrap, String> {
    if let Some(bootstrap) =
        cached_workspace_bootstrap_with_fresh_assistant_config(lookup.workspace.name())
    {
        return Ok(bootstrap);
    }

    let project_name = lookup.workspace.project().ok_or_else(|| {
        format!(
            "workspace {} is missing a project label",
            lookup.workspace.name()
        )
    })?;
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project = config.projects.get(project_name).ok_or_else(|| {
        format!(
            "project not found for workspace {}: {project_name}",
            lookup.workspace.name()
        )
    })?;

    let target_branch = if lookup.workspace.is_template() {
        project.target_branch.trim().to_string()
    } else {
        lookup
            .workspace
            .target_branch()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(project.target_branch.as_str())
            .trim()
            .to_string()
    };
    if target_branch.is_empty() {
        return Err(format!("project {project_name} is missing a target branch"));
    }

    let workspace_branch = if lookup.workspace.is_template() {
        None
    } else {
        let branch = lookup
            .workspace
            .branch_name()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if branch.is_empty() {
            return Err(format!(
                "workspace {} is missing branch metadata",
                lookup.workspace.name()
            ));
        }
        Some(branch)
    };

    build_workspace_bootstrap(
        &config,
        project_name,
        project,
        &target_branch,
        workspace_branch.as_deref(),
    )
}

fn workspace_bootstrap_script(lookup: &WorkspaceLookup, bootstrap: &WorkspaceBootstrap) -> String {
    let gh_hosts_yml = gh_hosts_yml(&bootstrap.gh_username, &bootstrap.gh_token);
    let bootstrap_signature = workspace_bootstrap_signature(lookup.workspace.name(), bootstrap);
    let agent_install = workspace_agent_install_script(lookup);
    let assistant_config_sync = workspace_assistant_config_sync_script(bootstrap);
    let cli_update = workspace_cli_update_script();
    let env_file_sync = if lookup.workspace.is_template() {
        workspace_env_file_sync_script(&bootstrap.env_files)
    } else {
        String::new()
    };
    let credentials_lines = [
        format!("export GH_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!("export GITHUB_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!(
            "export CLAUDE_CODE_OAUTH_TOKEN={}",
            shell_quote(&bootstrap.claude_token)
        ),
        "export GIT_ASKPASS=\"$HOME/.silo/git-askpass.sh\"".to_string(),
        "export GIT_TERMINAL_PROMPT=0".to_string(),
        "unset ANTHROPIC_API_KEY".to_string(),
        "unset ANTHROPIC_AUTH_TOKEN".to_string(),
        "unset ANTHROPIC_BASE_URL".to_string(),
        "unset CLAUDE_API_KEY".to_string(),
        "unset CLAUDE_CODE_USE_BEDROCK".to_string(),
        "unset CLAUDE_CODE_USE_VERTEX".to_string(),
        "unset AWS_BEARER_TOKEN_BEDROCK".to_string(),
        "unset VERTEX_REGION_CLAUDE_CODE".to_string(),
        format!(
            "if [[ -f {} ]]; then source {}; fi",
            shell_quote(REMOTE_WORKSPACE_AGENT_SHELL_FILE),
            shell_quote(REMOTE_WORKSPACE_AGENT_SHELL_FILE)
        ),
    ]
    .join("\n");
    let git_clone_target_branch = bootstrap_git_command(
        "clone --branch \"$TARGET_BRANCH\" \"$REMOTE_URL\" \"$WORKSPACE_DIR\"",
    );
    let git_fetch_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\"");
    let git_pull_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" pull --ff-only origin \"$TARGET_BRANCH\"");
    let git_lfs_setup = workspace_git_lfs_setup_script();
    let git_lfs_sync = workspace_git_lfs_sync_script();
    let branch_setup = if lookup.workspace.is_template() {
        format!(
            "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nelse\n  git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n  {git_fetch_target_branch}\n  git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" reset --hard \"origin/$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" clean -fd\nfi",
            git_clone_target_branch = git_clone_target_branch,
            git_fetch_target_branch = git_fetch_target_branch,
        )
    } else {
        branch_workspace_setup_script(
            &git_clone_target_branch,
            &git_fetch_target_branch,
            &git_pull_target_branch,
        )
    };

    format!(
        "set -euo pipefail\n\
WORKSPACE_DIR={workspace_dir}\n\
WORKSPACE_NAME={workspace_name}\n\
REMOTE_URL={remote_url}\n\
TARGET_BRANCH={target_branch}\n\
WORKSPACE_BRANCH={workspace_branch}\n\
GIT_USER_NAME={git_user_name}\n\
GIT_USER_EMAIL={git_user_email}\n\
mkdir -p \"$HOME/.silo\"\n\
chmod 700 \"$HOME/.silo\"\n\
LOCK_DIR={lock_dir}\n\
BOOTSTRAP_LOG_PATH={log_path}\n\
ASKPASS_PATH=\"$HOME/.silo/git-askpass.sh\"\n\
bootstrap_log() {{\n\
  local message=\"$1\"\n\
  local timestamp\n\
  timestamp=\"$(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || printf 'unknown-time')\"\n\
  printf '[%s][silo-bootstrap][%s] %s\\n' \"$timestamp\" \"$WORKSPACE_NAME\" \"$message\" | tee -a \"$BOOTSTRAP_LOG_PATH\" >&2\n\
}}\n\
cleanup() {{\n\
  rm -rf \"$LOCK_DIR\"\n\
}}\n\
for _ in $(seq 1 60); do\n\
  if mkdir \"$LOCK_DIR\" 2>/dev/null; then\n\
    trap cleanup EXIT\n\
    break\n\
  fi\n\
  sleep 1\n\
done\n\
if [ ! -d \"$LOCK_DIR\" ]; then\n\
  echo 'timed out waiting for workspace bootstrap lock' >&2\n\
  exit 1\n\
fi\n\
trap 'status=$?; if [ \"$status\" -ne 0 ]; then bootstrap_log \"failed exit_status=$status line=${{LINENO:-unknown}} command=${{BASH_COMMAND:-unknown}}\"; fi; cleanup' EXIT\n\
BOOT_ID=\"$(cat /proc/sys/kernel/random/boot_id)\"\n\
STATE_PATH={state_path}\n\
SIGNATURE={signature}\n\
if [ -f \"$STATE_PATH\" ]; then\n\
  CURRENT_BOOT_ID=\"$(sed -n '1p' \"$STATE_PATH\")\"\n\
  CURRENT_SIGNATURE=\"$(sed -n '2,$p' \"$STATE_PATH\")\"\n\
  if [ \"$CURRENT_BOOT_ID\" = \"$BOOT_ID\" ] && [ \"$CURRENT_SIGNATURE\" = \"$SIGNATURE\" ]; then\n\
    bootstrap_log 'bootstrap state already up to date'\n\
    exit 0\n\
  fi\n\
fi\n\
bootstrap_log 'step=write_credentials'\n\
cat > \"$ASKPASS_PATH\" <<'EOF_GIT_ASKPASS'\n\
#!/bin/sh\n\
case \"$1\" in\n\
  *Username*) printf '%s\\n' 'x-access-token' ;;\n\
  *Password*) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
  *) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
esac\n\
EOF_GIT_ASKPASS\n\
chmod 700 \"$ASKPASS_PATH\"\n\
cat > {credentials_path} <<'EOF'\n\
{credentials_lines}\n\
EOF\n\
chmod 600 {credentials_path}\n\
. {credentials_path}\n\
for rc in \"$HOME/.zshrc\" \"$HOME/.bashrc\"; do\n\
  touch \"$rc\"\n\
  if ! grep -Fqx '[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"' \"$rc\"; then\n\
    printf '\\n[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"\\n' >> \"$rc\"\n\
  fi\n\
done\n\
mkdir -p \"$HOME/.config/gh\"\n\
printf '%s\\n' {gh_hosts_yml} > \"$HOME/.config/gh/hosts.yml\"\n\
chmod 700 \"$HOME/.config\" \"$HOME/.config/gh\"\n\
chmod 600 \"$HOME/.config/gh/hosts.yml\"\n\
{assistant_config_sync}\
rm -f \"$HOME/.gitconfig.lock\"\n\
bootstrap_log 'step=configure_git'\n\
if [ -n \"$GIT_USER_NAME\" ] && [ \"$(git config --global --get user.name || true)\" != \"$GIT_USER_NAME\" ]; then\n\
  git config --global user.name \"$GIT_USER_NAME\"\n\
fi\n\
if [ -n \"$GIT_USER_EMAIL\" ] && [ \"$(git config --global --get user.email || true)\" != \"$GIT_USER_EMAIL\" ]; then\n\
  git config --global user.email \"$GIT_USER_EMAIL\"\n\
fi\n\
if ! git config --global --get-all safe.directory 2>/dev/null | grep -Fxq \"$WORKSPACE_DIR\"; then\n\
  git config --global --add safe.directory \"$WORKSPACE_DIR\"\n\
fi\n\
{git_lfs_setup}\
bootstrap_log 'step=sync_repository'\n\
{branch_setup}\n\
{git_lfs_sync}\
bootstrap_log 'step=sync_env_files'\n\
{env_file_sync}\n\
{cli_update}\
bootstrap_log 'step=install_workspace_agent'\n\
{agent_install}\
{state_write}\
bootstrap_log 'step=completed'\n",
        workspace_dir = shell_quote(TERMINAL_WORKSPACE_DIR),
        workspace_name = shell_quote(lookup.workspace.name()),
        remote_url = shell_quote(&bootstrap.remote_url),
        target_branch = shell_quote(&bootstrap.target_branch),
        workspace_branch = shell_quote(bootstrap.workspace_branch.as_deref().unwrap_or("")),
        git_user_name = shell_quote(&bootstrap.git_user_name),
        git_user_email = shell_quote(&bootstrap.git_user_email),
        lock_dir = shell_quote(REMOTE_BOOTSTRAP_LOCK_DIR),
        log_path = shell_quote(REMOTE_BOOTSTRAP_LOG_FILE),
        state_path = shell_quote(REMOTE_BOOTSTRAP_STATE_FILE),
        signature = shell_quote(&bootstrap_signature),
        credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE),
        credentials_lines = credentials_lines,
        gh_hosts_yml = shell_quote(&gh_hosts_yml),
        assistant_config_sync = assistant_config_sync,
        branch_setup = branch_setup,
        git_lfs_setup = git_lfs_setup,
        git_lfs_sync = git_lfs_sync,
        env_file_sync = env_file_sync,
        cli_update = cli_update,
        agent_install = agent_install,
        state_write = workspace_bootstrap_state_write_script(),
    )
}

fn branch_workspace_setup_script(
    git_clone_target_branch: &str,
    git_fetch_target_branch: &str,
    git_pull_target_branch: &str,
) -> String {
    format!(
        "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nfi\n\
git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n\
CURRENT_BRANCH=\"$(git -C \"$WORKSPACE_DIR\" symbolic-ref --quiet --short HEAD 2>/dev/null || printf '')\"\n\
WORKTREE_DIRTY=0\n\
if [ -n \"$(git -C \"$WORKSPACE_DIR\" status --porcelain --untracked-files=normal 2>/dev/null || printf '')\" ]; then\n\
  WORKTREE_DIRTY=1\n\
fi\n\
if git -C \"$WORKSPACE_DIR\" show-ref --verify --quiet \"refs/heads/$WORKSPACE_BRANCH\"; then\n\
  bootstrap_log \"workspace_branch_present branch=$WORKSPACE_BRANCH current_branch=${{CURRENT_BRANCH:-detached}}; skipping_target_sync\"\n\
elif [ \"$CURRENT_BRANCH\" != \"$TARGET_BRANCH\" ]; then\n\
  bootstrap_log \"workspace_branch_missing current_branch=${{CURRENT_BRANCH:-detached}} target_branch=$TARGET_BRANCH; skipping_target_sync\"\n\
elif [ \"$WORKTREE_DIRTY\" -ne 0 ]; then\n\
  bootstrap_log \"workspace_branch_missing current_branch=$CURRENT_BRANCH dirty_worktree=1; skipping_target_sync\"\n\
else\n\
  {git_fetch_target_branch}\n\
  {git_pull_target_branch}\n\
  git -C \"$WORKSPACE_DIR\" checkout -b \"$WORKSPACE_BRANCH\" \"$TARGET_BRANCH\"\n\
fi",
        git_clone_target_branch = git_clone_target_branch,
        git_fetch_target_branch = git_fetch_target_branch,
        git_pull_target_branch = git_pull_target_branch,
    )
}

fn workspace_bootstrap_state_write_script() -> &'static str {
    "{ printf '%s\\n' \"$BOOT_ID\"; printf '%s\\n' \"$SIGNATURE\"; } > \"$STATE_PATH\"\n\
chmod 600 \"$STATE_PATH\"\n\
"
}

pub(crate) fn bootstrap_git_command(command: &str) -> String {
    format!(
        "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git {command}"
    )
}

fn workspace_git_lfs_setup_script() -> String {
    String::from(
        "if git lfs version >/dev/null 2>&1; then\n\
  bootstrap_log 'step=configure_git_lfs'\n\
  git lfs install --skip-repo\n\
else\n\
  bootstrap_log 'step=configure_git_lfs_skipped reason=git_lfs_unavailable'\n\
fi\n",
    )
}

fn workspace_git_lfs_sync_script() -> String {
    let git_lfs_pull = bootstrap_git_command("-C \"$WORKSPACE_DIR\" lfs pull");
    format!(
        "if git lfs version >/dev/null 2>&1; then\n\
  bootstrap_log 'step=sync_git_lfs'\n\
  {git_lfs_pull}\n\
else\n\
  bootstrap_log 'step=sync_git_lfs_skipped reason=git_lfs_unavailable'\n\
fi\n"
    )
}

fn workspace_bootstrap_signature(workspace_name: &str, bootstrap: &WorkspaceBootstrap) -> String {
    format!(
        "version={}\nworkspace={}\nremote_url={}\ntarget_branch={}\nworkspace_branch={}\ngh_username={}\ngh_token_sha256={}\ncodex_auth_sha256={}\ncodex_model={}\ncodex_model_reasoning_effort={}\nclaude_token_sha256={}\nclaude_model={}\nclaude_effort_level={}\nclaude_always_thinking_enabled={}\ngit_user_name={}\ngit_user_email={}\nenv_files={}\nagent_sources={}",
        WORKSPACE_BOOTSTRAP_VERSION,
        workspace_name,
        bootstrap.remote_url,
        bootstrap.target_branch,
        bootstrap.workspace_branch.as_deref().unwrap_or(""),
        bootstrap.gh_username,
        hex_sha256(bootstrap.gh_token.as_bytes()),
        bootstrap.codex_auth_fingerprint,
        bootstrap.codex_model,
        bootstrap.codex_model_reasoning_effort,
        hex_sha256(bootstrap.claude_token.as_bytes()),
        bootstrap.claude_model,
        bootstrap.claude_effort_level,
        bootstrap.claude_always_thinking_enabled,
        bootstrap.git_user_name,
        bootstrap.git_user_email,
        bootstrap_env_files_signature(&bootstrap.env_files),
        workspace_agent_source_signature(),
    )
}

pub(crate) fn load_bootstrap_env_files(
    project_name: &str,
    project: &ProjectConfig,
) -> Vec<BootstrapEnvFile> {
    let project_root = Path::new(&project.path);
    let mut env_files = Vec::new();
    let mut seen = HashSet::new();

    for configured_path in &project.env_files {
        let Some(relative_path) = normalize_workspace_relative_path(configured_path) else {
            log::warn!(
                "skipping invalid env file path for project {}: {}",
                project_name,
                configured_path
            );
            continue;
        };

        if !seen.insert(relative_path.clone()) {
            continue;
        }

        let local_path = project_root.join(Path::new(&relative_path));
        let Ok(contents) = fs::read(&local_path) else {
            log::warn!(
                "skipping missing or unreadable env file for project {}: {}",
                project_name,
                local_path.display()
            );
            continue;
        };

        let contents_sha256 = hex_sha256(&contents);
        env_files.push(BootstrapEnvFile {
            relative_path,
            contents_base64: BASE64_STANDARD.encode(contents),
            contents_sha256,
        });
    }

    env_files
}

pub(crate) fn normalize_workspace_relative_path(path: &str) -> Option<String> {
    let mut normalized = String::new();

    for component in Path::new(path).components() {
        let Component::Normal(value) = component else {
            return None;
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(value.to_str()?);
    }

    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn bootstrap_env_files_signature(env_files: &[BootstrapEnvFile]) -> String {
    env_files
        .iter()
        .map(|env_file| format!("{}:{}", env_file.relative_path, env_file.contents_sha256))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn workspace_env_file_sync_script(env_files: &[BootstrapEnvFile]) -> String {
    if env_files.is_empty() {
        return String::new();
    }

    let mut script = String::from("# sync project env files into the template workspace\n");

    for (index, env_file) in env_files.iter().enumerate() {
        if let Some((parent_dir, _)) = env_file.relative_path.rsplit_once('/') {
            script.push_str(&format!(
                "mkdir -p {}\n",
                shell_quote(&format!("{TERMINAL_WORKSPACE_DIR}/{parent_dir}"))
            ));
        }

        let target_path = format!("{TERMINAL_WORKSPACE_DIR}/{}", env_file.relative_path);
        script.push_str(&format!(
            "cat <<'EOF_ENV_{index}' | base64 --decode > {target_path}\n{contents}\nEOF_ENV_{index}\nchmod 600 {target_path}\n",
            target_path = shell_quote(&target_path),
            contents = env_file.contents_base64,
        ));
    }

    script
}

fn workspace_assistant_config_sync_script(bootstrap: &WorkspaceBootstrap) -> String {
    let codex_auth_write = if bootstrap.codex_auth_json.is_empty() {
        "rm -f \"$HOME/.codex/auth.json\"".to_string()
    } else {
        format!(
            "printf '%s\\n' {} > \"$HOME/.codex/auth.json\"",
            shell_quote(&bootstrap.codex_auth_json)
        )
    };
    let codex_config_toml = codex_config_toml(
        &bootstrap.codex_model,
        &bootstrap.codex_model_reasoning_effort,
    );
    let claude_settings_json = claude_settings_json(
        &bootstrap.claude_model,
        &bootstrap.claude_effort_level,
        bootstrap.claude_always_thinking_enabled,
    );
    let claude_state_json = claude_state_json();

    format!(
        "mkdir -p \"$HOME/.codex\" \"$HOME/.claude\"\n\
{codex_auth_write}\n\
printf '%s\\n' {codex_config_toml} > \"$HOME/.codex/config.toml\"\n\
printf '%s\\n' {claude_settings_json} > \"$HOME/.claude/settings.json\"\n\
printf '%s\\n' {claude_state_json} > \"$HOME/.claude.json\"\n\
chmod 700 \"$HOME/.codex\" \"$HOME/.claude\"\n\
if [ -f \"$HOME/.codex/auth.json\" ]; then chmod 600 \"$HOME/.codex/auth.json\"; fi\n\
chmod 600 \"$HOME/.codex/config.toml\" \"$HOME/.claude/settings.json\" \"$HOME/.claude.json\"\n",
        codex_auth_write = codex_auth_write,
        codex_config_toml = shell_quote(&codex_config_toml),
        claude_settings_json = shell_quote(&claude_settings_json),
        claude_state_json = shell_quote(&claude_state_json),
    )
}

fn workspace_cli_update_script() -> String {
    String::from(
        "if command -v brew >/dev/null 2>&1; then\n\
  eval \"$(brew shellenv)\"\n\
  bootstrap_log 'step=brew_update'\n\
  brew update\n\
  bootstrap_log 'step=install_codex'\n\
  brew install codex\n\
fi\n\
if command -v curl >/dev/null 2>&1; then\n\
  bootstrap_log 'step=install_claude'\n\
  curl -fsSL https://claude.ai/install.sh | bash\n\
fi\n",
    )
}

fn workspace_agent_install_script(lookup: &WorkspaceLookup) -> String {
    workspace_agent_install_script_for_target(
        lookup.workspace.name(),
        &lookup.gcloud_project,
        lookup.workspace.zone(),
    )
}

fn workspace_agent_update_script(
    lookup: &WorkspaceLookup,
    bootstrap: Option<&WorkspaceBootstrap>,
) -> String {
    workspace_agent_update_script_for_target(
        lookup.workspace.name(),
        &lookup.gcloud_project,
        lookup.workspace.zone(),
        bootstrap,
    )
}

fn workspace_agent_update_script_for_target(
    instance: &str,
    project: &str,
    zone: &str,
    bootstrap: Option<&WorkspaceBootstrap>,
) -> String {
    let assistant_config_sync = bootstrap
        .map(workspace_assistant_config_sync_script)
        .unwrap_or_default();
    format!(
        "set -euo pipefail\n\
{assistant_config_sync}\
{agent_install}",
        assistant_config_sync = assistant_config_sync,
        agent_install = workspace_agent_install_script_for_target(instance, project, zone),
    )
}

fn workspace_agent_install_script_for_target(instance: &str, project: &str, zone: &str) -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_AGENT_BIN);
    let bin_tmp_path = shell_quote(&format!("{REMOTE_WORKSPACE_AGENT_BIN}.new"));
    let fingerprint_path = shell_quote(REMOTE_WORKSPACE_AGENT_FINGERPRINT_FILE);
    let fingerprint_tmp_path =
        shell_quote(&format!("{REMOTE_WORKSPACE_AGENT_FINGERPRINT_FILE}.new"));
    let shell_path = shell_quote(REMOTE_WORKSPACE_AGENT_SHELL_FILE);
    let shell_tmp_path = shell_quote(&format!("{REMOTE_WORKSPACE_AGENT_SHELL_FILE}.new"));
    let encoded_binary = BASE64_STANDARD.encode(WORKSPACE_AGENT_BIN_BYTES);
    let fingerprint = workspace_agent_fingerprint();
    let shell_script = workspace_agent_shell_script();
    let encoded_shell = BASE64_STANDARD.encode(shell_script.as_bytes());

    let mut script =
        "install -d -m 0700 \"$HOME/.silo\" \"$HOME/.silo/bin\" \"$HOME/.silo/workspace-agent\"\n"
            .to_string();
    script.push_str(&workspace_agent_stop_script());
    script.push_str(&format!(
        "cat <<'EOF_AGENT_BIN' | base64 --decode > {bin_tmp_path}\n{encoded_binary}\nEOF_AGENT_BIN\n",
        bin_tmp_path = bin_tmp_path,
    ));
    script.push_str(&format!(
        "chmod 0755 {bin_tmp_path}\n\
mv {bin_tmp_path} {bin_path}\n\
printf '%s\\n' {fingerprint} > {fingerprint_tmp_path}\n\
chmod 0600 {fingerprint_tmp_path}\n\
mv {fingerprint_tmp_path} {fingerprint_path}\n\
cat <<'EOF_AGENT_SHELL' | base64 --decode > {shell_tmp_path}\n{encoded_shell}\nEOF_AGENT_SHELL\n",
        bin_tmp_path = bin_tmp_path,
        bin_path = bin_path,
        fingerprint = shell_quote(&fingerprint),
        fingerprint_tmp_path = fingerprint_tmp_path,
        fingerprint_path = fingerprint_path,
        shell_tmp_path = shell_tmp_path,
    ));
    script.push_str(&format!(
        "chmod 0755 {shell_tmp_path}\n\
mv {shell_tmp_path} {shell_path}\n",
        shell_tmp_path = shell_tmp_path,
        shell_path = shell_path,
    ));
    script.push_str(&format!(
        "nohup {bin_path} daemon --instance {instance} --project {project} --zone {zone} >/dev/null 2>&1 < /dev/null &\n",
        instance = shell_quote(instance),
        project = shell_quote(project),
        zone = shell_quote(zone),
    ));
    script
}

fn workspace_agent_daemon_pid_discovery_script() -> String {
    let pidfile = shell_quote(REMOTE_WORKSPACE_AGENT_PIDFILE);
    let bin_path = shell_quote(REMOTE_WORKSPACE_AGENT_BIN);
    format!(
        "AGENT_PIDS=\"\"\n\
AGENT_BIN={bin_path}\n\
if [ -f {pidfile} ]; then\n\
  PID=\"$(cat {pidfile} 2>/dev/null || true)\"\n\
  if [ -n \"$PID\" ] && kill -0 \"$PID\" 2>/dev/null; then\n\
    ARGS=\"$(ps -o args= -p \"$PID\" 2>/dev/null || true)\"\n\
    case \"$ARGS\" in\n\
      \"$AGENT_BIN daemon\"|\"$AGENT_BIN daemon \"*) AGENT_PIDS=\"$PID\" ;;\n\
    esac\n\
  fi\n\
fi\n\
EXTRA_AGENT_PIDS=\"$(ps -eo pid=,args= 2>/dev/null | awk -v bin=\"$AGENT_BIN\" '$2 == bin && $3 == \"daemon\" {{ print $1 }}' || true)\"\n\
if [ -n \"$EXTRA_AGENT_PIDS\" ]; then\n\
  AGENT_PIDS=\"$AGENT_PIDS\n\
$EXTRA_AGENT_PIDS\"\n\
fi\n",
        bin_path = bin_path,
        pidfile = pidfile,
    )
}

fn workspace_agent_stop_script() -> String {
    let pidfile = shell_quote(REMOTE_WORKSPACE_AGENT_PIDFILE);
    format!(
        "{}for PID in $(printf '%s\\n' \"$AGENT_PIDS\" | awk 'NF' | sort -u); do\n\
  if kill -0 \"$PID\" 2>/dev/null; then\n\
    kill \"$PID\" 2>/dev/null || true\n\
    for _ in $(seq 1 50); do\n\
      if ! kill -0 \"$PID\" 2>/dev/null; then\n\
        break\n\
      fi\n\
      sleep 0.1\n\
    done\n\
    if kill -0 \"$PID\" 2>/dev/null; then\n\
      kill -9 \"$PID\" 2>/dev/null || true\n\
    fi\n\
  fi\n\
done\n\
rm -f {pidfile}\n",
        workspace_agent_daemon_pid_discovery_script(),
        pidfile = pidfile,
    )
}

fn workspace_agent_running_check_command() -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_AGENT_BIN);
    format!(
        "if [ ! -x {bin_path} ]; then\n\
  exit 1\n\
fi\n\
{}for PID in $(printf '%s\\n' \"$AGENT_PIDS\" | awk 'NF' | sort -u); do\n\
  exit 0\n\
done\n\
exit 1",
        workspace_agent_daemon_pid_discovery_script(),
        bin_path = bin_path,
    )
}

fn workspace_agent_running_check_remote_command() -> String {
    workspace_shell_command(&workspace_agent_running_check_command())
}

fn workspace_agent_ready_check_command() -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_AGENT_BIN);
    format!(
        "if [ ! -x {bin_path} ]; then\n\
  exit 1\n\
fi\n\
{bin_path} mark-read --session __silo_ready_probe__",
        bin_path = bin_path,
    )
}

fn workspace_agent_ready_check_remote_command() -> String {
    workspace_shell_command(&workspace_agent_ready_check_command())
}

fn workspace_agent_source_signature() -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_AGENT_BIN_BYTES);
    hasher.update(workspace_agent_shell_script().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn workspace_agent_shell_script() -> String {
    format!(
        "export SILO_WORKSPACE_AGENT_BIN={agent_bin}\n\
_silo_agent_emit() {{\n\
  [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
  [ -x \"${{SILO_WORKSPACE_AGENT_BIN:-}}\" ] || return 0\n\
  SILO_AGENT_HOOK=1 \"$SILO_WORKSPACE_AGENT_BIN\" emit \"$@\" >/dev/null 2>&1 || true\n\
}}\n\
_silo_agent_wrap_assistant() {{\n\
  local provider=\"$1\"\n\
  shift\n\
  if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_AGENT_BIN:-}}\" ]; then\n\
    command \"$@\"\n\
    return\n\
  fi\n\
  command \"$SILO_WORKSPACE_AGENT_BIN\" assistant-proxy --provider \"$provider\" -- \"$@\"\n\
}}\n\
codex() {{\n\
  _silo_agent_wrap_assistant codex codex \"$@\"\n\
}}\n\
claude() {{\n\
  _silo_agent_wrap_assistant claude claude --dangerously-skip-permissions \"$@\"\n\
}}\n\
cc() {{\n\
  claude \"$@\"\n\
}}\n\
silo() {{\n\
  local provider=\"${{1:-}}\"\n\
  shift || true\n\
  case \"$provider\" in\n\
    codex)\n\
      if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_AGENT_BIN:-}}\" ]; then\n\
        command codex \"$@\"\n\
        return\n\
      fi\n\
      command \"$SILO_WORKSPACE_AGENT_BIN\" assistant-proxy --provider codex --initial-prompt-argv -- codex \"$@\"\n\
      ;;\n\
    claude)\n\
      if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_AGENT_BIN:-}}\" ]; then\n\
        IS_SANDBOX=1 command claude --dangerously-skip-permissions \"$@\"\n\
        return\n\
      fi\n\
      IS_SANDBOX=1 command \"$SILO_WORKSPACE_AGENT_BIN\" assistant-proxy --provider claude --initial-prompt-argv -- claude --dangerously-skip-permissions \"$@\"\n\
      ;;\n\
    *)\n\
      printf 'unsupported silo assistant: %s\\n' \"$provider\" >&2\n\
      return 1\n\
      ;;\n\
  esac\n\
}}\n\
case $- in\n\
  *i*) ;;\n\
  *) return 0 2>/dev/null || exit 0 ;;\n\
esac\n\
if [ -n \"${{ZMX_SESSION:-}}\" ] && [ -z \"${{SILO_AGENT_SESSION_REGISTERED:-}}\" ]; then\n\
  export SILO_AGENT_SESSION_REGISTERED=1\n\
  _silo_agent_emit --kind shell_session_started --session \"$ZMX_SESSION\"\n\
fi\n\
if [ -n \"${{ZSH_VERSION:-}}\" ]; then\n\
  autoload -Uz add-zsh-hook\n\
  typeset -g SILO_AGENT_LAST_COMMAND=\"${{SILO_AGENT_LAST_COMMAND:-}}\"\n\
  _silo_agent_preexec() {{\n\
    [ -n \"${{SILO_AGENT_HOOK:-}}\" ] && return 0\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    SILO_AGENT_LAST_COMMAND=\"$1\"\n\
    _silo_agent_emit --kind shell_command_started --session \"$ZMX_SESSION\" --command \"$1\"\n\
  }}\n\
  _silo_agent_precmd() {{\n\
    local exit_code=$?\n\
    [ -n \"${{SILO_AGENT_HOOK:-}}\" ] && return $exit_code\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return $exit_code\n\
    if [ -n \"${{SILO_AGENT_LAST_COMMAND:-}}\" ]; then\n\
      _silo_agent_emit --kind shell_command_finished --session \"$ZMX_SESSION\"\n\
      SILO_AGENT_LAST_COMMAND=\"\"\n\
    fi\n\
    return $exit_code\n\
  }}\n\
  _silo_agent_zshexit() {{\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    [ -n \"${{SILO_AGENT_SESSION_REGISTERED:-}}\" ] || return 0\n\
    _silo_agent_emit --kind shell_session_exited --session \"$ZMX_SESSION\"\n\
  }}\n\
  case \" ${{preexec_functions[*]:-}} \" in\n\
    *\" _silo_agent_preexec \"*) ;;\n\
    *) add-zsh-hook preexec _silo_agent_preexec ;;\n\
  esac\n\
  case \" ${{precmd_functions[*]:-}} \" in\n\
    *\" _silo_agent_precmd \"*) ;;\n\
    *) add-zsh-hook precmd _silo_agent_precmd ;;\n\
  esac\n\
  case \" ${{zshexit_functions[*]:-}} \" in\n\
    *\" _silo_agent_zshexit \"*) ;;\n\
    *) add-zsh-hook zshexit _silo_agent_zshexit ;;\n\
  esac\n\
elif [ -n \"${{BASH_VERSION:-}}\" ]; then\n\
  SILO_AGENT_LAST_COMMAND=\"${{SILO_AGENT_LAST_COMMAND:-}}\"\n\
  SILO_AGENT_BASH_IN_PROMPT=0\n\
  _silo_agent_bash_preexec() {{\n\
    local exit_code=$?\n\
    [ -n \"${{SILO_AGENT_HOOK:-}}\" ] && return $exit_code\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return $exit_code\n\
    [ \"${{SILO_AGENT_BASH_IN_PROMPT:-0}}\" = \"1\" ] && return $exit_code\n\
    case \"$BASH_COMMAND\" in\n\
      _silo_agent_*|trap*|history*|\"PROMPT_COMMAND=\"*) return $exit_code ;;\n\
    esac\n\
    if [ -n \"${{SILO_AGENT_LAST_COMMAND:-}}\" ] && [ \"$BASH_COMMAND\" = \"$SILO_AGENT_LAST_COMMAND\" ]; then\n\
      return $exit_code\n\
    fi\n\
    SILO_AGENT_LAST_COMMAND=\"$BASH_COMMAND\"\n\
    _silo_agent_emit --kind shell_command_started --session \"$ZMX_SESSION\" --command \"$BASH_COMMAND\"\n\
    return $exit_code\n\
  }}\n\
  _silo_agent_bash_precmd() {{\n\
    local exit_code=$?\n\
    SILO_AGENT_BASH_IN_PROMPT=1\n\
    [ -n \"${{SILO_AGENT_HOOK:-}}\" ] && {{ SILO_AGENT_BASH_IN_PROMPT=0; return $exit_code; }}\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || {{ SILO_AGENT_BASH_IN_PROMPT=0; return $exit_code; }}\n\
    if [ -n \"${{SILO_AGENT_LAST_COMMAND:-}}\" ]; then\n\
      _silo_agent_emit --kind shell_command_finished --session \"$ZMX_SESSION\"\n\
      SILO_AGENT_LAST_COMMAND=\"\"\n\
    fi\n\
    SILO_AGENT_BASH_IN_PROMPT=0\n\
    return $exit_code\n\
  }}\n\
  _silo_agent_bash_exit() {{\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    [ -n \"${{SILO_AGENT_SESSION_REGISTERED:-}}\" ] || return 0\n\
    _silo_agent_emit --kind shell_session_exited --session \"$ZMX_SESSION\"\n\
  }}\n\
  trap _silo_agent_bash_preexec DEBUG\n\
  case \";${{PROMPT_COMMAND:-}};\" in\n\
    *\";_silo_agent_bash_precmd;\"*) ;;\n\
    *) PROMPT_COMMAND=\"_silo_agent_bash_precmd${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;;\n\
  esac\n\
  trap _silo_agent_bash_exit EXIT\n\
fi\n",
        agent_bin = shell_quote(REMOTE_WORKSPACE_AGENT_BIN),
    )
}

fn codex_auth_fingerprint(auth_json: &str) -> String {
    codex_token_from_auth_json(auth_json)
        .map(|token| hex_sha256(token.as_bytes()))
        .unwrap_or_default()
}

fn codex_config_toml(model: &str, model_reasoning_effort: &str) -> String {
    format!(
        r#"personality = "pragmatic"
model = {model:?}
model_reasoning_effort = {model_reasoning_effort:?}
approval_policy = "never"
sandbox_mode = "danger-full-access"
suppress_unstable_features_warning = true

[projects."/home/silo/workspace"]
trust_level = "trusted"

[notice]
hide_full_access_warning = true
"#,
    )
}

fn claude_settings_json(model: &str, effort_level: &str, always_thinking_enabled: bool) -> String {
    json!({
        "model": model,
        "alwaysThinkingEnabled": always_thinking_enabled,
        "effortLevel": effort_level,
        "skipDangerousModePermissionPrompt": true
    })
    .to_string()
}

pub(crate) fn claude_state_json() -> String {
    json!({
        "installMethod": "native",
        "autoUpdates": false,
        "hasCompletedOnboarding": true,
        "projects": {
            TERMINAL_WORKSPACE_DIR: {
                "allowedTools": [],
                "mcpContextUris": [],
                "mcpServers": {},
                "enabledMcpjsonServers": [],
                "disabledMcpjsonServers": [],
                "hasTrustDialogAccepted": true,
                "projectOnboardingSeenCount": 1,
                "hasCompletedProjectOnboarding": true,
                "hasClaudeMdExternalIncludesApproved": false,
                "hasClaudeMdExternalIncludesWarningShown": false
            }
        }
    })
    .to_string()
}

fn gh_hosts_yml(username: &str, token: &str) -> String {
    format!(
        "github.com:\n    user: {username}\n    oauth_token: {token}\n    git_protocol: https\n"
    )
}
pub(crate) fn clear_template_runtime_state_command() -> String {
    format!(
        "set -e\n{}rm -rf \"$HOME/.silo\"",
        workspace_agent_stop_script()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use std::env;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn workspace_agent_shell_script_emits_shell_session_lifecycle_events() {
        let script = workspace_agent_shell_script();

        assert!(script.contains("--kind shell_session_started"));
        assert!(script.contains("--kind shell_session_exited"));
        assert!(script.contains("SILO_AGENT_SESSION_REGISTERED"));
        assert!(script.contains("add-zsh-hook zshexit _silo_agent_zshexit"));
    }

    #[test]
    fn codex_config_toml_uses_configured_model_and_reasoning() {
        let config = codex_config_toml("gpt-5.4", "xhigh");

        assert!(config.contains("model = \"gpt-5.4\""));
        assert!(config.contains("model_reasoning_effort = \"xhigh\""));
        assert!(config.contains("suppress_unstable_features_warning = true"));
        assert!(!config.contains("codex_hooks = true"));
    }

    #[test]
    fn claude_settings_json_uses_configured_model_and_effort() {
        let settings = claude_settings_json("opus", "high", true);

        assert!(settings.contains("\"model\":\"opus\""));
        assert!(settings.contains("\"effortLevel\":\"high\""));
        assert!(settings.contains("\"alwaysThinkingEnabled\":true"));
    }

    #[test]
    fn bootstrap_git_command_inlines_auth_env() {
        assert_eq!(
            bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""),
            "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git -C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""
        );
    }

    #[test]
    fn workspace_git_lfs_setup_script_configures_lfs_when_available() {
        let script = workspace_git_lfs_setup_script();

        assert!(script.contains("bootstrap_log 'step=configure_git_lfs'"));
        assert!(script.contains("git lfs install --skip-repo"));
        assert!(script.contains("git_lfs_unavailable"));
    }

    #[test]
    fn workspace_git_lfs_sync_script_pulls_lfs_objects_when_available() {
        let script = workspace_git_lfs_sync_script();

        assert!(script.contains("bootstrap_log 'step=sync_git_lfs'"));
        assert!(script.contains("git -C \"$WORKSPACE_DIR\" lfs pull"));
        assert!(script.contains("GIT_ASKPASS=\"$ASKPASS_PATH\""));
        assert!(script.contains("git_lfs_unavailable"));
    }

    #[test]
    fn branch_workspace_setup_script_skips_target_checkout_for_existing_branches() {
        let script = branch_workspace_setup_script(
            "git clone --branch \"$TARGET_BRANCH\" \"$REMOTE_URL\" \"$WORKSPACE_DIR\"",
            "git -C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\"",
            "git -C \"$WORKSPACE_DIR\" pull --ff-only origin \"$TARGET_BRANCH\"",
        );

        assert!(script.contains(
            "git -C \"$WORKSPACE_DIR\" show-ref --verify --quiet \"refs/heads/$WORKSPACE_BRANCH\""
        ));
        assert!(script.contains("workspace_branch_present branch=$WORKSPACE_BRANCH"));
        assert!(!script.contains("git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\""));
    }

    #[test]
    fn branch_workspace_setup_script_only_syncs_clean_target_branch_worktrees() {
        let script = branch_workspace_setup_script(
            "git clone --branch \"$TARGET_BRANCH\" \"$REMOTE_URL\" \"$WORKSPACE_DIR\"",
            "git -C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\"",
            "git -C \"$WORKSPACE_DIR\" pull --ff-only origin \"$TARGET_BRANCH\"",
        );

        assert!(script.contains(
            "CURRENT_BRANCH=\"$(git -C \"$WORKSPACE_DIR\" symbolic-ref --quiet --short HEAD"
        ));
        assert!(script.contains("status --porcelain --untracked-files=normal"));
        assert!(script.contains("elif [ \"$CURRENT_BRANCH\" != \"$TARGET_BRANCH\" ]; then"));
        assert!(script.contains("elif [ \"$WORKTREE_DIRTY\" -ne 0 ]; then"));
        assert!(script.contains(
            "git -C \"$WORKSPACE_DIR\" checkout -b \"$WORKSPACE_BRANCH\" \"$TARGET_BRANCH\""
        ));
    }

    #[test]
    fn bootstrap_retry_detects_broken_pipe() {
        assert!(should_retry_template_bootstrap(
            "failed to write gcloud ssh stdin: Broken pipe (os error 32)"
        ));
    }

    #[test]
    fn transport_retry_detects_local_address_rebind_failure() {
        assert!(is_retryable_terminal_transport_error(
            "Read from remote host 35.245.135.222: Can't assign requested address"
        ));
    }

    #[test]
    fn transport_retry_ignores_missing_remote_session_errors() {
        assert!(!is_retryable_terminal_transport_error(
            "zmx attach: session not found"
        ));
    }

    #[test]
    fn workspace_agent_install_script_stops_agent_before_replacing_binary() {
        let script = workspace_agent_install_script_for_target(
            "demo-silo-alpha",
            "demo-project",
            "us-east4-c",
        );
        let stop_index = script
            .find("kill \"$PID\"")
            .expect("script should stop an existing agent before replacing it");
        let write_index = script
            .find("base64 --decode > '/home/silo/.silo/bin/workspace-agent.new'")
            .expect("script should write the replacement agent to a temp file");
        let move_index = script
            .find("mv '/home/silo/.silo/bin/workspace-agent.new' '/home/silo/.silo/bin/workspace-agent'")
            .expect("script should atomically replace the agent binary");

        assert!(stop_index < write_index);
        assert!(write_index < move_index);
    }

    #[test]
    fn workspace_agent_stop_script_targets_daemon_processes_only() {
        let script = workspace_agent_stop_script();

        assert!(script.contains("$AGENT_BIN daemon"));
        assert!(script.contains("$3 == \"daemon\""));
        assert!(!script.contains("pgrep -x workspace-agent"));
    }

    #[test]
    fn workspace_agent_install_script_writes_fingerprint_file() {
        let script = workspace_agent_install_script_for_target(
            "demo-silo-alpha",
            "demo-project",
            "us-east4-c",
        );

        assert!(script.contains(REMOTE_WORKSPACE_AGENT_FINGERPRINT_FILE));
        assert!(script.contains("workspace-agent/fingerprint.new"));
        assert!(script.contains("printf '%s\\n'"));
        assert!(script.contains(&workspace_agent_fingerprint()));
    }

    #[test]
    fn workspace_agent_running_check_command_stays_small_and_only_checks_state() {
        let command = workspace_agent_running_check_command();

        assert!(command.contains("$AGENT_BIN daemon"));
        assert!(command.contains("ps -eo pid=,args="));
        assert!(command.contains("/home/silo/.silo/workspace-agent/daemon.pid"));
        assert!(!command.contains("EOF_AGENT_BIN"));
        assert!(!command.contains("workspace-agent.new"));
    }

    #[test]
    fn workspace_agent_running_check_remote_command_uses_encoded_workspace_shell() {
        let command = workspace_agent_running_check_remote_command();

        assert!(command.contains("base64 --decode | bash"));
        assert!(command.contains("printf %s"));
        assert!(!command.contains("sudo -iu silo bash -lc 'if ["));
    }

    #[test]
    fn workspace_agent_ready_check_command_probes_fifo_delivery() {
        let command = workspace_agent_ready_check_command();

        assert!(command.contains("mark-read --session __silo_ready_probe__"));
        assert!(!command.contains("ps -eo pid=,args="));
        assert!(!command.contains("workspace-agent.new"));
    }

    #[test]
    fn workspace_agent_ready_check_remote_command_uses_encoded_workspace_shell() {
        let command = workspace_agent_ready_check_remote_command();

        assert!(command.contains("base64 --decode | bash"));
        assert!(command.contains("printf %s"));
        assert!(!command.contains("sudo -iu silo bash -lc 'if ["));
    }

    #[test]
    fn workspace_agent_wait_timeout_error_prefers_probe_failure() {
        let error = workspace_agent_wait_timeout_error(
            "demo-silo-alpha",
            Some("fingerprint"),
            true,
            true,
            Some("workspace agent readiness probe failed: failed to open fifo"),
        );

        assert_eq!(
            error,
            "workspace agent for demo-silo-alpha did not accept the readiness probe: workspace agent readiness probe failed: failed to open fifo"
        );
    }

    #[test]
    fn workspace_agent_wait_timeout_error_reports_expected_fingerprint_when_never_seen() {
        let error = workspace_agent_wait_timeout_error(
            "demo-silo-alpha",
            Some("fingerprint"),
            true,
            false,
            None,
        );

        assert_eq!(
            error,
            "workspace agent for demo-silo-alpha did not publish the expected fingerprint"
        );
    }

    #[test]
    fn stale_agent_update_state_requires_running_workspace() {
        assert!(!workspace_has_stale_agent_update_state_with_expected(
            "TERMINATED",
            "updating_workspace_agent",
            Some("fingerprint"),
            "fingerprint",
        ));
    }

    #[test]
    fn stale_agent_update_state_requires_matching_fingerprint() {
        assert!(!workspace_has_stale_agent_update_state_with_expected(
            "RUNNING",
            "updating_workspace_agent",
            Some("old-fingerprint"),
            "new-fingerprint",
        ));
    }

    #[test]
    fn stale_agent_update_state_detects_expected_fingerprint_stuck_updating() {
        assert!(workspace_has_stale_agent_update_state_with_expected(
            "RUNNING",
            "updating_workspace_agent",
            Some("fingerprint"),
            "fingerprint",
        ));
    }

    #[test]
    fn workspace_bootstrap_state_write_script_persists_boot_id_and_signature() {
        let script = workspace_bootstrap_state_write_script();

        assert!(script.contains("> \"$STATE_PATH\""));
        assert!(script.contains("printf '%s\\n' \"$BOOT_ID\""));
        assert!(script.contains("printf '%s\\n' \"$SIGNATURE\""));
        assert!(script.contains("chmod 600 \"$STATE_PATH\""));
    }

    #[test]
    fn clear_template_runtime_state_command_stops_agent_and_removes_remote_silo_dir() {
        let command = clear_template_runtime_state_command();

        assert!(command.starts_with("set -e\nAGENT_PIDS="));
        assert!(command.contains("$3 == \"daemon\""));
        assert!(command.contains("rm -f '/home/silo/.silo/workspace-agent/daemon.pid'"));
        assert!(command.ends_with("rm -rf \"$HOME/.silo\""));
    }

    #[test]
    fn workspace_agent_release_rollout_state_round_trips() {
        let _guard = ENV_LOCK.lock().expect("env lock should be available");
        let temp_state_dir = env::temp_dir().join(format!(
            "silo-workspace-agent-rollout-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_state_dir).expect("temp state dir should exist");
        let previous = env::var_os(state_paths::SILO_STATE_DIR_ENV_VAR);
        env::set_var(state_paths::SILO_STATE_DIR_ENV_VAR, &temp_state_dir);

        let expected = WorkspaceAgentReleaseRolloutState {
            app_version: "2026.83.1".to_string(),
            fingerprint: "abc123".to_string(),
            completed_at: "2026-03-23T00:00:00Z".to_string(),
        };

        save_workspace_agent_release_rollout_state(&expected).expect("rollout state should save");
        let actual = load_workspace_agent_release_rollout_state()
            .expect("rollout state should load")
            .expect("rollout state should exist");
        assert_eq!(actual, expected);

        if let Some(previous) = previous {
            env::set_var(state_paths::SILO_STATE_DIR_ENV_VAR, previous);
        } else {
            env::remove_var(state_paths::SILO_STATE_DIR_ENV_VAR);
        }
        let _ = fs::remove_dir_all(&temp_state_dir);
    }

    #[test]
    fn normalize_workspace_relative_path_rejects_non_relative_paths() {
        assert_eq!(
            normalize_workspace_relative_path("apps/web/.env.local"),
            Some("apps/web/.env.local".to_string())
        );
        assert_eq!(
            normalize_workspace_relative_path(".env"),
            Some(".env".to_string())
        );
        assert_eq!(normalize_workspace_relative_path("../.env"), None);
        assert_eq!(normalize_workspace_relative_path("/tmp/.env"), None);
    }

    #[test]
    fn load_bootstrap_env_files_reads_and_hashes_configured_files() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join("apps/web")).expect("nested env dir should exist");
        fs::write(project_root.join(".env.local"), "ROOT=1\n").expect("root env should exist");
        fs::write(project_root.join("apps/web/.env"), "WEB=1\n").expect("nested env should exist");

        let project = ProjectConfig {
            name: "demo".to_string(),
            path: project_root.to_string_lossy().into_owned(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            env_files: vec![
                ".env.local".to_string(),
                "apps/web/.env".to_string(),
                "../ignored".to_string(),
            ],
            gcloud: Default::default(),
        };

        let env_files = load_bootstrap_env_files("demo", &project);

        assert_eq!(env_files.len(), 2);
        assert_eq!(env_files[0].relative_path, ".env.local");
        assert_eq!(env_files[0].contents_sha256, hex_sha256(b"ROOT=1\n"));
        assert_eq!(env_files[1].relative_path, "apps/web/.env");
        assert_eq!(env_files[1].contents_sha256, hex_sha256(b"WEB=1\n"));
    }

    #[test]
    fn workspace_env_file_sync_script_preserves_relative_paths() {
        let script = workspace_env_file_sync_script(&[BootstrapEnvFile {
            relative_path: "apps/web/.env.local".to_string(),
            contents_base64: BASE64_STANDARD.encode("WEB=1\n"),
            contents_sha256: "digest".to_string(),
        }]);

        assert!(script.contains("mkdir -p '/home/silo/workspace/apps/web'"));
        assert!(script.contains("base64 --decode > '/home/silo/workspace/apps/web/.env.local'"));
        assert!(script.contains("chmod 600 '/home/silo/workspace/apps/web/.env.local'"));
    }

    #[test]
    fn workspace_cli_update_script_refreshes_codex_and_claude() {
        let script = workspace_cli_update_script();

        assert!(script.contains("bootstrap_log 'step=brew_update'"));
        assert!(script.contains("bootstrap_log 'step=install_codex'"));
        assert!(script.contains("bootstrap_log 'step=install_claude'"));
        assert!(script.contains("brew update"));
        assert!(script.contains("brew install codex"));
        assert!(script.contains("https://claude.ai/install.sh"));
    }

    #[test]
    fn workspace_assistant_config_sync_script_writes_assistant_config_files() {
        let script = workspace_assistant_config_sync_script(&WorkspaceBootstrap {
            remote_url: "https://github.com/example/repo.git".to_string(),
            target_branch: "main".to_string(),
            workspace_branch: Some("feature/demo".to_string()),
            gh_username: "octocat".to_string(),
            gh_token: "gh-secret".to_string(),
            codex_auth_json: "{\"tokens\":{\"refresh_token\":\"codex-secret\"}}".to_string(),
            codex_auth_fingerprint: hex_sha256(b"codex-secret"),
            codex_model: "gpt-5.4".to_string(),
            codex_model_reasoning_effort: "xhigh".to_string(),
            claude_token: "claude-secret".to_string(),
            claude_model: "opus".to_string(),
            claude_effort_level: "high".to_string(),
            claude_always_thinking_enabled: true,
            git_user_name: "Example User".to_string(),
            git_user_email: "user@example.com".to_string(),
            env_files: Vec::new(),
        });

        assert!(!script.contains("$HOME/.codex/hooks.json"));
        assert!(script.contains("$HOME/.claude/settings.json"));
        assert!(script.contains("model = \"gpt-5.4\""));
        assert!(script.contains("\"model\":\"opus\""));
        assert!(!script.contains("codex_hooks = true"));
    }

    #[test]
    fn workspace_agent_update_script_skips_assistant_config_when_bootstrap_is_unavailable() {
        let script = workspace_agent_update_script_for_target(
            "demo-silo-feature",
            "silo-489618",
            "us-east4-c",
            None,
        );

        assert!(script.starts_with("set -euo pipefail\n"));
        assert!(script.contains("EOF_AGENT_BIN"));
        assert!(!script.contains("$HOME/.codex/config.toml"));
        assert!(!script.contains("$HOME/.claude/settings.json"));
    }

    #[test]
    fn workspace_agent_update_script_includes_assistant_config_when_bootstrap_is_available() {
        let bootstrap = WorkspaceBootstrap {
            remote_url: "https://github.com/example/repo.git".to_string(),
            target_branch: "main".to_string(),
            workspace_branch: Some("feature/demo".to_string()),
            gh_username: "octocat".to_string(),
            gh_token: "gh-secret".to_string(),
            codex_auth_json: "{\"tokens\":{\"refresh_token\":\"codex-secret\"}}".to_string(),
            codex_auth_fingerprint: hex_sha256(b"codex-secret"),
            codex_model: "gpt-5.4".to_string(),
            codex_model_reasoning_effort: "xhigh".to_string(),
            claude_token: "claude-secret".to_string(),
            claude_model: "opus".to_string(),
            claude_effort_level: "high".to_string(),
            claude_always_thinking_enabled: true,
            git_user_name: "Example User".to_string(),
            git_user_email: "user@example.com".to_string(),
            env_files: Vec::new(),
        };

        let script = workspace_agent_update_script_for_target(
            "demo-silo-feature",
            "silo-489618",
            "us-east4-c",
            Some(&bootstrap),
        );

        assert!(script.contains("$HOME/.codex/config.toml"));
        assert!(script.contains("$HOME/.claude/settings.json"));
        assert!(script.contains("EOF_AGENT_BIN"));
    }

    #[test]
    fn workspace_bootstrap_signature_hashes_secrets() {
        let signature = workspace_bootstrap_signature(
            "demo-silo-template",
            &WorkspaceBootstrap {
                remote_url: "https://github.com/example/repo.git".to_string(),
                target_branch: "staging".to_string(),
                workspace_branch: None,
                gh_username: "octocat".to_string(),
                gh_token: "gh-secret".to_string(),
                codex_auth_json: "{\"tokens\":{\"refresh_token\":\"codex-secret\"}}".to_string(),
                codex_auth_fingerprint: hex_sha256(b"codex-secret"),
                codex_model: "gpt-5.4".to_string(),
                codex_model_reasoning_effort: "xhigh".to_string(),
                claude_token: "claude-secret".to_string(),
                claude_model: "opus".to_string(),
                claude_effort_level: "high".to_string(),
                claude_always_thinking_enabled: true,
                git_user_name: "Example User".to_string(),
                git_user_email: "user@example.com".to_string(),
                env_files: Vec::new(),
            },
        );

        assert!(signature.contains("gh_token_sha256="));
        assert!(signature.contains("codex_auth_sha256="));
        assert!(signature.contains("codex_model=gpt-5.4"));
        assert!(signature.contains("codex_model_reasoning_effort=xhigh"));
        assert!(signature.contains("claude_token_sha256="));
        assert!(signature.contains("claude_model=opus"));
        assert!(signature.contains("claude_effort_level=high"));
        assert!(signature.contains("claude_always_thinking_enabled=true"));
        assert!(!signature.contains("gh-secret"));
        assert!(!signature.contains("codex-secret"));
        assert!(!signature.contains("claude-secret"));
    }

    #[test]
    fn codex_auth_fingerprint_hashes_refresh_token() {
        let payload = "{\"tokens\":{\"access_token\":\"codex-access-token\",\"refresh_token\":\"codex-refresh-token\"}}";
        assert_eq!(
            codex_auth_fingerprint(payload),
            hex_sha256(b"codex-refresh-token")
        );
    }

    #[test]
    fn claude_state_json_marks_workspace_as_trusted_and_onboarded() {
        let payload = claude_state_json();
        assert!(payload.contains(TERMINAL_WORKSPACE_DIR));
        assert!(payload.contains("\"hasTrustDialogAccepted\":true"));
        assert!(payload.contains("\"hasCompletedOnboarding\":true"));
    }

    #[test]
    fn cache_workspace_bootstrap_stores_seeded_bootstrap() {
        let config = SiloConfig {
            git: crate::config::GitConfig {
                gh_username: "octocat".to_string(),
                gh_token: "gh-secret".to_string(),
                user_name: "Example User".to_string(),
                user_email: "user@example.com".to_string(),
            },
            codex: crate::config::CodexConfig {
                auth_json: "{\"tokens\":{\"refresh_token\":\"codex-refresh-token\"}}".to_string(),
                ..crate::config::CodexConfig::default()
            },
            claude: crate::config::ClaudeConfig {
                token: "claude-secret".to_string(),
                ..crate::config::ClaudeConfig::default()
            },
            projects: IndexMap::new(),
            ..SiloConfig::default()
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            env_files: Vec::new(),
            gcloud: crate::config::ProjectGcloudConfig::default(),
        };

        cache_workspace_bootstrap(
            "demo-silo-branch",
            &config,
            "demo",
            &project,
            "main",
            Some("feature/demo"),
        )
        .expect("bootstrap cache should seed");

        let bootstrap = cached_workspace_bootstrap("demo-silo-branch")
            .expect("bootstrap seed should be cached");
        assert_eq!(bootstrap.remote_url, "git@github.com:example/demo.git");
        assert_eq!(bootstrap.target_branch, "main");
        assert_eq!(bootstrap.workspace_branch.as_deref(), Some("feature/demo"));
        assert_eq!(bootstrap.gh_username, "octocat");
        assert_eq!(
            bootstrap.codex_auth_fingerprint,
            hex_sha256(b"codex-refresh-token")
        );
        assert_eq!(bootstrap.codex_model, "gpt-5.4");
        assert_eq!(bootstrap.codex_model_reasoning_effort, "xhigh");
        assert_eq!(bootstrap.claude_model, "opus");
        assert_eq!(bootstrap.claude_effort_level, "high");
        assert!(bootstrap.claude_always_thinking_enabled);
    }

    #[test]
    fn cached_workspace_bootstrap_refreshes_assistant_preferences_from_updated_config() {
        let _guard = ENV_LOCK.lock().expect("env lock should be available");
        let temp_dir = TempDir::new();
        let previous_home = env::var_os("HOME");
        env::set_var("HOME", &temp_dir.path);

        let stale_config = SiloConfig {
            git: crate::config::GitConfig {
                gh_username: "octocat".to_string(),
                gh_token: "gh-secret".to_string(),
                user_name: "Example User".to_string(),
                user_email: "user@example.com".to_string(),
            },
            codex: crate::config::CodexConfig {
                auth_json: "{\"tokens\":{\"refresh_token\":\"stale-codex-refresh-token\"}}"
                    .to_string(),
                model: "gpt-5.4".to_string(),
                model_reasoning_effort: "high".to_string(),
            },
            claude: crate::config::ClaudeConfig {
                token: "claude-secret".to_string(),
                model: "opus".to_string(),
                effort_level: "high".to_string(),
                always_thinking_enabled: true,
            },
            projects: IndexMap::new(),
            ..SiloConfig::default()
        };
        let fresh_config = SiloConfig {
            git: stale_config.git.clone(),
            codex: crate::config::CodexConfig {
                auth_json: "{\"tokens\":{\"refresh_token\":\"fresh-codex-refresh-token\"}}"
                    .to_string(),
                model: "gpt-5.4-mini".to_string(),
                model_reasoning_effort: "xhigh".to_string(),
            },
            claude: crate::config::ClaudeConfig {
                token: "claude-secret".to_string(),
                model: "sonnet".to_string(),
                effort_level: "medium".to_string(),
                always_thinking_enabled: false,
            },
            projects: IndexMap::new(),
            ..SiloConfig::default()
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            env_files: Vec::new(),
            gcloud: crate::config::ProjectGcloudConfig::default(),
        };

        let store = ConfigStore::from_home_dir(temp_dir.path.clone());
        store
            .save(&stale_config)
            .expect("stale config should be persisted");
        cache_workspace_bootstrap(
            "demo-silo-auth-refresh",
            &stale_config,
            "demo",
            &project,
            "main",
            Some("feature/demo"),
        )
        .expect("bootstrap cache should seed");
        store
            .save(&fresh_config)
            .expect("fresh config should be persisted");

        let bootstrap =
            cached_workspace_bootstrap_with_fresh_assistant_config("demo-silo-auth-refresh")
                .expect("cached bootstrap should exist");
        assert_eq!(
            bootstrap.codex_auth_json,
            "{\"tokens\":{\"refresh_token\":\"fresh-codex-refresh-token\"}}"
        );
        assert_eq!(
            bootstrap.codex_auth_fingerprint,
            hex_sha256(b"fresh-codex-refresh-token")
        );
        assert_eq!(bootstrap.codex_model, "gpt-5.4-mini");
        assert_eq!(bootstrap.codex_model_reasoning_effort, "xhigh");
        assert_eq!(bootstrap.claude_model, "sonnet");
        assert_eq!(bootstrap.claude_effort_level, "medium");
        assert!(!bootstrap.claude_always_thinking_enabled);

        let persisted = store.load().expect("synced config should load");
        assert_eq!(persisted.codex.auth_json, fresh_config.codex.auth_json);
        assert_eq!(persisted.codex.model, fresh_config.codex.model);
        assert_eq!(
            persisted.codex.model_reasoning_effort,
            fresh_config.codex.model_reasoning_effort
        );
        assert_eq!(persisted.claude.model, fresh_config.claude.model);
        assert_eq!(
            persisted.claude.effort_level,
            fresh_config.claude.effort_level
        );
        assert_eq!(
            persisted.claude.always_thinking_enabled,
            fresh_config.claude.always_thinking_enabled
        );

        if let Some(previous_home) = previous_home {
            env::set_var("HOME", previous_home);
        } else {
            env::remove_var("HOME");
        }
        if let Ok(mut states) = WORKSPACE_STARTUP_RECONCILE_STATE.lock() {
            states.remove("demo-silo-auth-refresh");
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = format!(
                "silo-bootstrap-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(Path::new(&self.path));
        }
    }
}
