use crate::bootstrap;
use crate::config::ConfigStore;
use crate::state::WorkspaceMetadataManager;
use crate::workspaces::{
    self, ResolvedGcloudConfig, Snapshot, SnapshotTemplate, TemplateWorkspace, WorkspaceLookup,
    TEMPLATE_OPERATION_ID_LABEL_KEY, TEMPLATE_OPERATION_KIND_LABEL_KEY,
    TEMPLATE_OPERATION_PHASE_LABEL_KEY,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use tauri::State;

const LEGACY_TEMPLATE_STATE_FILE_NAME: &str = "template-state.json";
const LEGACY_TEMPLATE_STATE_DIR_NAME: &str = ".silo";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOperationKind {
    Create,
    Edit,
    Save,
    Delete,
}

impl TemplateOperationKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Edit => "edit",
            Self::Save => "save",
            Self::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOperationStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateOperation {
    pub project: String,
    pub workspace_name: String,
    pub kind: TemplateOperationKind,
    pub status: TemplateOperationStatus,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_name: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateState {
    pub project: String,
    pub workspace_name: String,
    pub workspace_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<TemplateOperation>,
}

#[tauri::command]
pub async fn templates_list_templates() -> Result<Vec<SnapshotTemplate>, String> {
    workspaces::list_template_snapshots().await
}

#[tauri::command]
pub async fn templates_get_state(
    project: String,
    manager: State<'_, WorkspaceMetadataManager>,
) -> Result<TemplateState, String> {
    let state = derive_template_state(&project, manager.inner()).await?;
    maybe_start_operation_reconcile(&state, manager.inner());
    Ok(state)
}

#[tauri::command]
pub async fn templates_create_template(
    project: String,
    manager: State<'_, WorkspaceMetadataManager>,
) -> Result<TemplateWorkspace, String> {
    manager.clear_transient_template_state(&project);

    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace =
        ensure_template_workspace_for_operation(&project, TemplateOperationKind::Create).await?;
    bootstrap::start_template_bootstrap(workspace_name);

    Ok(workspace)
}

#[tauri::command]
pub async fn templates_edit_template(
    project: String,
    manager: State<'_, WorkspaceMetadataManager>,
) -> Result<TemplateWorkspace, String> {
    manager.clear_transient_template_state(&project);

    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace =
        ensure_template_workspace_for_operation(&project, TemplateOperationKind::Edit).await?;
    bootstrap::start_template_bootstrap(workspace_name);

    Ok(workspace)
}

#[tauri::command]
pub async fn templates_save_template(
    project: String,
    manager: State<'_, WorkspaceMetadataManager>,
) -> Result<TemplateOperation, String> {
    manager.clear_transient_template_state(&project);

    let lookup = find_template_workspace_lookup(&project)
        .await?
        .ok_or_else(|| format!("template workspace not found for project {project}"))?;
    let snapshot_name = lookup
        .workspace
        .template_operation()
        .and_then(|operation| {
            (operation.kind == TemplateOperationKind::Save.as_str())
                .then(|| operation.snapshot_name.clone())
                .flatten()
        })
        .unwrap_or_else(|| workspaces::generate_template_snapshot_name(&project));

    enqueue_template_operation(
        manager.inner(),
        &lookup,
        TemplateOperationKind::Save.as_str(),
        "waiting_for_template_ready",
        Some("Waiting for template workspace bootstrap"),
        None,
        Some(&snapshot_name),
    );

    let operation = TemplateOperation {
        project: project.clone(),
        workspace_name: workspaces::generate_template_workspace_name(&project),
        kind: TemplateOperationKind::Save,
        status: TemplateOperationStatus::Running,
        phase: "waiting_for_template_ready".to_string(),
        detail: Some("Waiting for template workspace bootstrap".to_string()),
        last_error: None,
        snapshot_name: Some(snapshot_name),
        updated_at: workspaces::current_rfc3339_timestamp(),
    };

    start_template_operation_reconcile_if_needed(project, manager.inner().clone());
    Ok(operation)
}

#[tauri::command]
pub async fn templates_delete_template(
    project: String,
    manager: State<'_, WorkspaceMetadataManager>,
) -> Result<TemplateOperation, String> {
    manager.clear_transient_template_state(&project);

    let gcloud = resolve_project_gcloud_config(&project)?;
    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace_lookup = find_template_workspace_lookup(&project).await?;

    if let Some(lookup) = workspace_lookup {
        enqueue_template_operation(
            manager.inner(),
            &lookup,
            TemplateOperationKind::Delete.as_str(),
            "deleting_snapshots",
            Some("Deleting template snapshots"),
            None,
            None,
        );
    } else {
        let snapshots = workspaces::list_template_snapshots_in_project(
            &gcloud.account,
            &gcloud.project,
            &project,
        )
        .await?;
        if snapshots.is_empty() {
            return Err(format!("template not found for project {project}"));
        }

        let operation_id = template_operation_id();
        tag_snapshots_for_delete(&gcloud, &snapshots, &operation_id, "deleting-snapshots").await?;
    }

    let operation = TemplateOperation {
        project: project.clone(),
        workspace_name,
        kind: TemplateOperationKind::Delete,
        status: TemplateOperationStatus::Running,
        phase: "deleting_snapshots".to_string(),
        detail: Some("Deleting template snapshots".to_string()),
        last_error: None,
        snapshot_name: None,
        updated_at: workspaces::current_rfc3339_timestamp(),
    };

    start_template_operation_reconcile_if_needed(project, manager.inner().clone());
    Ok(operation)
}

pub(crate) fn remove_legacy_template_state_file() {
    let Some(home_dir) = env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let path = home_dir
        .join(LEGACY_TEMPLATE_STATE_DIR_NAME)
        .join(LEGACY_TEMPLATE_STATE_FILE_NAME);
    if !path.exists() {
        return;
    }

    match fs::remove_file(&path) {
        Ok(()) => log::info!("removed legacy template state file {}", path.display()),
        Err(error) => log::warn!(
            "failed to remove legacy template state file {}: {}",
            path.display(),
            error
        ),
    }
}

pub(crate) fn resume_running_template_operations(manager: WorkspaceMetadataManager) {
    tauri::async_runtime::spawn(async move {
        let config = match ConfigStore::new().and_then(|store| store.load()) {
            Ok(config) => config,
            Err(error) => {
                log::warn!("failed to load config while resuming template operations: {error}");
                return;
            }
        };

        for project in config.projects.keys() {
            match derive_template_state(project, &manager).await {
                Ok(state) => maybe_start_operation_reconcile(&state, &manager),
                Err(error) => {
                    log::warn!(
                        "failed to derive template state while resuming project {}: {}",
                        project,
                        error
                    );
                }
            }
        }
    });
}

fn maybe_start_operation_reconcile(state: &TemplateState, manager: &WorkspaceMetadataManager) {
    if state
        .operation
        .as_ref()
        .is_some_and(|operation| operation.status == TemplateOperationStatus::Running)
    {
        start_template_operation_reconcile_if_needed(state.project.clone(), manager.clone());
    }
}

fn enqueue_template_operation(
    manager: &WorkspaceMetadataManager,
    lookup: &WorkspaceLookup,
    kind: &str,
    phase: &str,
    detail: Option<&str>,
    last_error: Option<&str>,
    snapshot_name: Option<&str>,
) {
    manager.enqueue_template_operation(
        lookup.workspace.name(),
        Some(lookup.clone()),
        kind,
        phase,
        detail,
        last_error,
        snapshot_name,
    );
}

pub(crate) fn start_template_operation_reconcile_if_needed(
    project: String,
    manager: WorkspaceMetadataManager,
) {
    if !manager.begin_template_reconcile(&project) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let result = reconcile_template_operation(&project, &manager).await;
        if let Err(error) = result {
            log::warn!(
                "template operation reconcile failed for project {}: {}",
                project,
                error
            );
            if let Err(failure_error) =
                record_template_operation_failure(&project, &manager, &error).await
            {
                log::warn!(
                    "failed to record template operation failure for project {}: {}",
                    project,
                    failure_error
                );
            }
        }

        manager.finish_template_reconcile(&project);
    });
}

async fn derive_template_state(
    project: &str,
    manager: &WorkspaceMetadataManager,
) -> Result<TemplateState, String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let workspace = find_template_workspace(&workspace_name, &gcloud).await?;
    let snapshots =
        workspaces::list_template_snapshots_in_project(&gcloud.account, &gcloud.project, project)
            .await?;
    let snapshot_name = snapshots
        .iter()
        .find(|snapshot| snapshot.status == "READY")
        .map(|snapshot| snapshot.name.clone());

    let operation = workspace
        .as_ref()
        .and_then(|workspace| derive_workspace_operation(project, &workspace_name, workspace))
        .or_else(|| derive_snapshot_operation(project, &workspace_name, &snapshots));

    let mut state = TemplateState {
        project: project.to_string(),
        workspace_name,
        workspace_present: workspace.is_some(),
        snapshot_name,
        operation,
    };

    if state.operation.is_none() {
        if let Some(cached) = manager.recent_transient_template_state(project) {
            state.operation = cached.operation;
            if state.snapshot_name.is_none() {
                state.snapshot_name = cached.snapshot_name;
            }
        }
    }

    Ok(state)
}

fn derive_workspace_operation(
    project: &str,
    workspace_name: &str,
    workspace: &TemplateWorkspace,
) -> Option<TemplateOperation> {
    let operation = workspace.template_operation()?;
    let kind = parse_operation_kind(&operation.kind)?;
    let status = if operation.phase == "failed" {
        TemplateOperationStatus::Failed
    } else {
        TemplateOperationStatus::Running
    };

    Some(TemplateOperation {
        project: project.to_string(),
        workspace_name: workspace_name.to_string(),
        kind,
        status,
        phase: operation.phase.clone(),
        detail: operation.detail.clone(),
        last_error: operation.last_error.clone(),
        snapshot_name: operation.snapshot_name.clone(),
        updated_at: operation
            .updated_at
            .clone()
            .unwrap_or_else(workspaces::current_rfc3339_timestamp),
    })
}

fn derive_snapshot_operation(
    project: &str,
    workspace_name: &str,
    snapshots: &[Snapshot],
) -> Option<TemplateOperation> {
    let snapshot = snapshots
        .iter()
        .find(|snapshot| snapshot.template_operation_kind.as_deref() == Some("delete"))?;
    let phase = snapshot
        .template_operation_phase
        .clone()
        .unwrap_or_else(|| "deleting_snapshots".to_string())
        .replace('-', "_");
    let status = if phase == "failed" {
        TemplateOperationStatus::Failed
    } else {
        TemplateOperationStatus::Running
    };

    Some(TemplateOperation {
        project: project.to_string(),
        workspace_name: workspace_name.to_string(),
        kind: TemplateOperationKind::Delete,
        status: status.clone(),
        phase,
        detail: Some(match status {
            TemplateOperationStatus::Failed => "Template operation failed".to_string(),
            TemplateOperationStatus::Running => "Deleting template snapshots".to_string(),
            TemplateOperationStatus::Completed => "Template deleted".to_string(),
        }),
        last_error: None,
        snapshot_name: None,
        updated_at: workspaces::current_rfc3339_timestamp(),
    })
}

async fn reconcile_template_operation(
    project: &str,
    manager: &WorkspaceMetadataManager,
) -> Result<(), String> {
    let state = derive_template_state(project, manager).await?;
    let Some(operation) = state.operation else {
        return Ok(());
    };
    if operation.status != TemplateOperationStatus::Running {
        return Ok(());
    }

    match operation.kind {
        TemplateOperationKind::Save => {
            reconcile_template_save_operation(project, &operation, manager).await
        }
        TemplateOperationKind::Delete => {
            reconcile_template_delete_operation(project, &operation, manager).await
        }
        TemplateOperationKind::Create | TemplateOperationKind::Edit => Ok(()),
    }
}

async fn reconcile_template_save_operation(
    project: &str,
    operation: &TemplateOperation,
    manager: &WorkspaceMetadataManager,
) -> Result<(), String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let snapshot_name = operation
        .snapshot_name
        .clone()
        .ok_or_else(|| format!("template save missing snapshot name for project {project}"))?;

    if save_phase_rank(&operation.phase) <= save_phase_rank("clearing_runtime_state") {
        bootstrap::wait_for_template_bootstrap(&workspace_name).await?;
        let lookup = find_template_workspace_lookup(project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;
        enqueue_template_operation(
            manager,
            &lookup,
            TemplateOperationKind::Save.as_str(),
            "clearing_runtime_state",
            Some("Removing template-only runtime state"),
            None,
            Some(&snapshot_name),
        );
        bootstrap::clear_template_runtime_state(&workspace_name).await?;
    }

    let mut boot_disk = None;
    if save_phase_rank(&operation.phase) <= save_phase_rank("stopping_vm") {
        let lookup = find_template_workspace_lookup(project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;
        enqueue_template_operation(
            manager,
            &lookup,
            TemplateOperationKind::Save.as_str(),
            "stopping_vm",
            Some("Stopping template virtual machine"),
            None,
            Some(&snapshot_name),
        );
        boot_disk = Some(
            workspaces::ensure_template_workspace_terminated(
                &gcloud.account,
                &gcloud.project,
                &workspace_name,
                &gcloud.zone,
            )
            .await?,
        );
    }

    if save_phase_rank(&operation.phase) <= save_phase_rank("creating_snapshot") {
        let lookup = find_template_workspace_lookup(project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;
        enqueue_template_operation(
            manager,
            &lookup,
            TemplateOperationKind::Save.as_str(),
            "creating_snapshot",
            Some("Creating template snapshot"),
            None,
            Some(&snapshot_name),
        );
        if workspaces::describe_snapshot_if_exists_in_project(
            &snapshot_name,
            &gcloud.account,
            &gcloud.project,
        )
        .await?
        .is_none()
        {
            workspaces::create_template_snapshot_for_disk_named(
                &gcloud.account,
                &gcloud.project,
                project,
                &snapshot_name,
                boot_disk.as_deref().ok_or_else(|| {
                    format!("template save missing boot disk for project {project}")
                })?,
                &gcloud.zone,
            )
            .await?;
        }
    }

    if save_phase_rank(&operation.phase) <= save_phase_rank("waiting_for_snapshot_ready") {
        let lookup = find_template_workspace_lookup(project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;
        enqueue_template_operation(
            manager,
            &lookup,
            TemplateOperationKind::Save.as_str(),
            "waiting_for_snapshot_ready",
            Some("Waiting for template snapshot"),
            None,
            Some(&snapshot_name),
        );
        workspaces::wait_for_template_snapshot_ready(
            &gcloud.account,
            &gcloud.project,
            &snapshot_name,
        )
        .await?;
    }

    if save_phase_rank(&operation.phase) <= save_phase_rank("deleting_old_snapshots") {
        let lookup = find_template_workspace_lookup(project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;
        enqueue_template_operation(
            manager,
            &lookup,
            TemplateOperationKind::Save.as_str(),
            "deleting_old_snapshots",
            Some("Removing previous template snapshots"),
            None,
            Some(&snapshot_name),
        );
        workspaces::delete_old_template_snapshots(
            &gcloud.account,
            &gcloud.project,
            project,
            &snapshot_name,
        )
        .await?;
    }

    let lookup = find_template_workspace_lookup(project)
        .await?
        .ok_or_else(|| format!("template workspace not found for project {project}"))?;
    enqueue_template_operation(
        manager,
        &lookup,
        TemplateOperationKind::Save.as_str(),
        "deleting_template_workspace",
        Some("Deleting template workspace"),
        None,
        Some(&snapshot_name),
    );
    workspaces::delete_template_workspace_if_exists(
        &gcloud.account,
        &gcloud.project,
        &workspace_name,
        &gcloud.zone,
    )
    .await?;

    manager.cache_transient_template_state(TemplateState {
        project: project.to_string(),
        workspace_name,
        workspace_present: false,
        snapshot_name: Some(snapshot_name.clone()),
        operation: Some(TemplateOperation {
            project: project.to_string(),
            workspace_name: workspaces::generate_template_workspace_name(project),
            kind: TemplateOperationKind::Save,
            status: TemplateOperationStatus::Completed,
            phase: "completed".to_string(),
            detail: Some("Template saved".to_string()),
            last_error: None,
            snapshot_name: Some(snapshot_name),
            updated_at: workspaces::current_rfc3339_timestamp(),
        }),
    });

    Ok(())
}

async fn reconcile_template_delete_operation(
    project: &str,
    operation: &TemplateOperation,
    manager: &WorkspaceMetadataManager,
) -> Result<(), String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let workspace_lookup = find_template_workspace_lookup(project).await?;

    match workspace_lookup {
        Some(lookup) => {
            if delete_phase_rank(&operation.phase) <= delete_phase_rank("deleting_snapshots") {
                enqueue_template_operation(
                    manager,
                    &lookup,
                    TemplateOperationKind::Delete.as_str(),
                    "deleting_snapshots",
                    Some("Deleting template snapshots"),
                    None,
                    None,
                );
                workspaces::delete_template_snapshots(&gcloud.account, &gcloud.project, project)
                    .await?;
            }

            enqueue_template_operation(
                manager,
                &lookup,
                TemplateOperationKind::Delete.as_str(),
                "deleting_template_workspace",
                Some("Deleting template workspace"),
                None,
                None,
            );
            workspaces::delete_template_workspace_if_exists(
                &gcloud.account,
                &gcloud.project,
                &workspace_name,
                &gcloud.zone,
            )
            .await?;
        }
        None => {
            let snapshots = workspaces::list_template_snapshots_in_project(
                &gcloud.account,
                &gcloud.project,
                project,
            )
            .await?;
            let delete_snapshots = snapshots
                .iter()
                .filter(|snapshot| snapshot.template_operation_kind.as_deref() == Some("delete"))
                .map(|snapshot| snapshot.name.clone())
                .collect::<Vec<_>>();
            if !delete_snapshots.is_empty() {
                for snapshot_name in delete_snapshots {
                    let result = workspaces::run_gcloud(
                        &gcloud.account,
                        &gcloud.project,
                        workspaces::delete_snapshot_args(&snapshot_name),
                    )
                    .await?;
                    if !result.success {
                        return Err(format!(
                            "failed to delete template snapshot {}: {}",
                            snapshot_name,
                            result.stderr.trim()
                        ));
                    }
                }
            }
        }
    }

    manager.cache_transient_template_state(TemplateState {
        project: project.to_string(),
        workspace_name,
        workspace_present: false,
        snapshot_name: None,
        operation: Some(TemplateOperation {
            project: project.to_string(),
            workspace_name: workspaces::generate_template_workspace_name(project),
            kind: TemplateOperationKind::Delete,
            status: TemplateOperationStatus::Completed,
            phase: "completed".to_string(),
            detail: Some("Template deleted".to_string()),
            last_error: None,
            snapshot_name: None,
            updated_at: workspaces::current_rfc3339_timestamp(),
        }),
    });

    Ok(())
}

async fn record_template_operation_failure(
    project: &str,
    manager: &WorkspaceMetadataManager,
    error: &str,
) -> Result<(), String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);

    if let Some(lookup) = find_template_workspace_lookup(project).await? {
        let kind = lookup
            .workspace
            .template_operation()
            .map(|operation| operation.kind.clone())
            .unwrap_or_else(|| "save".to_string());
        let snapshot_name = lookup
            .workspace
            .template_operation()
            .and_then(|operation| operation.snapshot_name.clone());
        enqueue_template_operation(
            manager,
            &lookup,
            &kind,
            "failed",
            Some("Template operation failed"),
            Some(error),
            snapshot_name.as_deref(),
        );
        return Ok(());
    }

    let snapshots =
        workspaces::list_template_snapshots_in_project(&gcloud.account, &gcloud.project, project)
            .await?;
    let delete_snapshots = snapshots
        .iter()
        .filter(|snapshot| snapshot.template_operation_kind.as_deref() == Some("delete"))
        .cloned()
        .collect::<Vec<_>>();
    if !delete_snapshots.is_empty() {
        let operation_id = delete_snapshots
            .first()
            .and_then(|snapshot| snapshot.template_operation_id.clone())
            .unwrap_or_else(template_operation_id);
        tag_snapshots_for_delete(&gcloud, &delete_snapshots, &operation_id, "failed").await?;
        manager.cache_transient_template_state(TemplateState {
            project: project.to_string(),
            workspace_name,
            workspace_present: false,
            snapshot_name: None,
            operation: Some(TemplateOperation {
                project: project.to_string(),
                workspace_name: workspaces::generate_template_workspace_name(project),
                kind: TemplateOperationKind::Delete,
                status: TemplateOperationStatus::Failed,
                phase: "failed".to_string(),
                detail: Some("Template operation failed".to_string()),
                last_error: Some(error.to_string()),
                snapshot_name: None,
                updated_at: workspaces::current_rfc3339_timestamp(),
            }),
        });
    }

    Ok(())
}

async fn ensure_template_workspace_for_operation(
    project: &str,
    kind: TemplateOperationKind,
) -> Result<TemplateWorkspace, String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    if let Some(existing) = find_template_workspace(&workspace_name, &gcloud).await? {
        return Ok(existing);
    }

    match kind {
        TemplateOperationKind::Create => {
            workspaces::create_template_workspace_for_project(project, None).await
        }
        TemplateOperationKind::Edit => {
            let snapshot_name = workspaces::latest_template_snapshot_name(
                &gcloud.account,
                &gcloud.project,
                project,
            )
            .await?
            .ok_or_else(|| format!("template not found for project {project}"))?;
            workspaces::create_template_workspace_for_project(project, Some(snapshot_name)).await
        }
        TemplateOperationKind::Save | TemplateOperationKind::Delete => Err(format!(
            "unsupported template workspace ensure operation for project {project}"
        )),
    }
}

async fn find_template_workspace(
    workspace_name: &str,
    gcloud: &ResolvedGcloudConfig,
) -> Result<Option<TemplateWorkspace>, String> {
    let workspace =
        workspaces::find_workspace_in_project(workspace_name, &gcloud.account, &gcloud.project)
            .await?;
    let Some(workspace) = workspace else {
        return Ok(None);
    };

    match workspace {
        crate::workspaces::Workspace::Template(workspace) => Ok(Some(workspace)),
        crate::workspaces::Workspace::Branch(_) => Err(format!(
            "workspace name is already in use for template workspace {workspace_name}"
        )),
    }
}

async fn find_template_workspace_lookup(
    project: &str,
) -> Result<Option<crate::workspaces::WorkspaceLookup>, String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let workspace =
        workspaces::find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project)
            .await?;
    let Some(workspace) = workspace else {
        return Ok(None);
    };

    match workspace {
        crate::workspaces::Workspace::Template(workspace) => {
            Ok(Some(crate::workspaces::WorkspaceLookup {
                workspace: crate::workspaces::Workspace::Template(workspace),
                account: gcloud.account,
                gcloud_project: gcloud.project,
            }))
        }
        crate::workspaces::Workspace::Branch(_) => Err(format!(
            "workspace name is already in use for project {project}: {workspace_name}"
        )),
    }
}

fn resolve_project_gcloud_config(project: &str) -> Result<ResolvedGcloudConfig, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    workspaces::resolve_project_gcloud_config(&config, project)
}

fn parse_operation_kind(value: &str) -> Option<TemplateOperationKind> {
    match value {
        "create" => Some(TemplateOperationKind::Create),
        "edit" => Some(TemplateOperationKind::Edit),
        "save" => Some(TemplateOperationKind::Save),
        "delete" => Some(TemplateOperationKind::Delete),
        _ => None,
    }
}

fn template_operation_id() -> String {
    workspaces::current_rfc3339_timestamp()
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect()
}

async fn tag_snapshots_for_delete(
    gcloud: &ResolvedGcloudConfig,
    snapshots: &[Snapshot],
    operation_id: &str,
    phase: &str,
) -> Result<(), String> {
    for snapshot in snapshots {
        workspaces::update_template_snapshot_labels(
            &gcloud.account,
            &gcloud.project,
            &snapshot.name,
            &[
                (TEMPLATE_OPERATION_KIND_LABEL_KEY, "delete"),
                (TEMPLATE_OPERATION_PHASE_LABEL_KEY, phase),
                (TEMPLATE_OPERATION_ID_LABEL_KEY, operation_id),
            ],
            &[],
        )
        .await?;
    }

    Ok(())
}

fn save_phase_rank(phase: &str) -> usize {
    match phase {
        "waiting_for_template_ready" => 0,
        "clearing_runtime_state" => 1,
        "stopping_vm" => 2,
        "creating_snapshot" => 3,
        "waiting_for_snapshot_ready" => 4,
        "deleting_old_snapshots" => 5,
        "deleting_template_workspace" => 6,
        "completed" => 7,
        _ => 0,
    }
}

fn delete_phase_rank(phase: &str) -> usize {
    match phase {
        "deleting_snapshots" => 0,
        "deleting_template_workspace" => 1,
        "completed" => 2,
        _ => 0,
    }
}
