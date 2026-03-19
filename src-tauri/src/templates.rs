use crate::bootstrap;
use crate::config::ConfigStore;
use crate::workspaces::{
    self, ResolvedGcloudConfig, SnapshotTemplate, TemplateWorkspace, Workspace,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

const TEMPLATE_STATE_FILE_NAME: &str = "template-state.json";
const TEMPLATE_STATE_DIR_NAME: &str = ".silo";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateOperationKind {
    Create,
    Edit,
    Save,
    Delete,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TemplateOperationFile {
    operations: HashMap<String, TemplateOperation>,
}

#[derive(Clone)]
pub struct TemplateOperationManager {
    inner: Arc<TemplateOperationManagerInner>,
}

struct TemplateOperationManagerInner {
    state_path: PathBuf,
    operations: Mutex<HashMap<String, TemplateOperation>>,
    in_flight: Mutex<HashSet<String>>,
}

impl TemplateOperationManager {
    pub fn load() -> Result<Self, String> {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME environment variable is not set".to_string())?;
        let state_path = home_dir
            .join(TEMPLATE_STATE_DIR_NAME)
            .join(TEMPLATE_STATE_FILE_NAME);
        let operations = load_operations(&state_path)?;

        Ok(Self {
            inner: Arc::new(TemplateOperationManagerInner {
                state_path,
                operations: Mutex::new(operations),
                in_flight: Mutex::new(HashSet::new()),
            }),
        })
    }

    pub fn resume_running_operations(&self) {
        let running_projects = self
            .inner
            .operations
            .lock()
            .map(|operations| {
                operations
                    .values()
                    .filter(|operation| operation.status == TemplateOperationStatus::Running)
                    .map(|operation| operation.project.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for project in running_projects {
            self.start_reconcile(&project);
        }
    }

    fn get_operation(&self, project: &str) -> Result<Option<TemplateOperation>, String> {
        self.inner
            .operations
            .lock()
            .map(|operations| operations.get(project).cloned())
            .map_err(|_| "template operation lock poisoned".to_string())
    }

    fn set_operation(&self, operation: TemplateOperation) -> Result<(), String> {
        let mut operations = self
            .inner
            .operations
            .lock()
            .map_err(|_| "template operation lock poisoned".to_string())?;
        operations.insert(operation.project.clone(), operation);
        persist_operations(&self.inner.state_path, &operations)
    }

    fn mutate_operation<F>(&self, project: &str, mutator: F) -> Result<TemplateOperation, String>
    where
        F: FnOnce(&mut TemplateOperation),
    {
        let mut operations = self
            .inner
            .operations
            .lock()
            .map_err(|_| "template operation lock poisoned".to_string())?;
        let operation = operations
            .get_mut(project)
            .ok_or_else(|| format!("template operation missing for project {project}"))?;
        mutator(operation);
        operation.updated_at = workspaces::current_rfc3339_timestamp();
        let operation = operation.clone();
        persist_operations(&self.inner.state_path, &operations)?;
        Ok(operation)
    }

    fn begin_operation(
        &self,
        project: &str,
        kind: TemplateOperationKind,
        phase: &str,
        detail: Option<&str>,
    ) -> Result<TemplateOperation, String> {
        let operation = TemplateOperation {
            project: project.to_string(),
            workspace_name: workspaces::generate_template_workspace_name(project),
            kind,
            status: TemplateOperationStatus::Running,
            phase: phase.to_string(),
            detail: detail.map(str::to_string),
            last_error: None,
            snapshot_name: None,
            updated_at: workspaces::current_rfc3339_timestamp(),
        };
        self.set_operation(operation.clone())?;
        Ok(operation)
    }

    fn update_phase(
        &self,
        project: &str,
        phase: &str,
        detail: Option<&str>,
    ) -> Result<TemplateOperation, String> {
        self.mutate_operation(project, |operation| {
            operation.phase = phase.to_string();
            operation.detail = detail.map(str::to_string);
            operation.last_error = None;
        })
    }

    fn update_snapshot(
        &self,
        project: &str,
        snapshot_name: &str,
    ) -> Result<TemplateOperation, String> {
        self.mutate_operation(project, |operation| {
            operation.snapshot_name = Some(snapshot_name.to_string());
        })
    }

    fn complete_operation(
        &self,
        project: &str,
        phase: &str,
        detail: Option<&str>,
    ) -> Result<TemplateOperation, String> {
        self.mutate_operation(project, |operation| {
            operation.status = TemplateOperationStatus::Completed;
            operation.phase = phase.to_string();
            operation.detail = detail.map(str::to_string);
            operation.last_error = None;
        })
    }

    fn fail_operation(
        &self,
        project: &str,
        phase: &str,
        error: &str,
    ) -> Result<TemplateOperation, String> {
        self.mutate_operation(project, |operation| {
            operation.status = TemplateOperationStatus::Failed;
            operation.phase = phase.to_string();
            operation.detail = Some("Template operation failed".to_string());
            operation.last_error = Some(error.to_string());
        })
    }

    fn start_reconcile(&self, project: &str) {
        let inserted = self
            .inner
            .in_flight
            .lock()
            .map(|mut in_flight| in_flight.insert(project.to_string()))
            .unwrap_or(false);
        if !inserted {
            return;
        }

        let manager = self.clone();
        let project = project.to_string();
        tauri::async_runtime::spawn(async move {
            let result = reconcile_template_operation(manager.clone(), &project).await;
            if let Err(error) = result {
                log::warn!(
                    "template operation reconcile failed for project {}: {}",
                    project,
                    error
                );
                let failed_phase = manager
                    .get_operation(&project)
                    .ok()
                    .flatten()
                    .map(|operation| operation.phase)
                    .unwrap_or_else(|| "failed".to_string());
                let _ = manager.fail_operation(&project, &failed_phase, &error);
            }

            if let Ok(mut in_flight) = manager.inner.in_flight.lock() {
                in_flight.remove(&project);
            }
        });
    }
}

#[tauri::command]
pub async fn templates_list_templates() -> Result<Vec<SnapshotTemplate>, String> {
    workspaces::list_template_snapshots().await
}

#[tauri::command]
pub async fn templates_get_state(
    project: String,
    manager: State<'_, TemplateOperationManager>,
) -> Result<TemplateState, String> {
    let gcloud = resolve_project_gcloud_config(&project)?;
    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace_present =
        workspaces::find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project)
            .await?
            .is_some();
    let snapshot_name =
        workspaces::latest_template_snapshot_name(&gcloud.account, &gcloud.project, &project)
            .await?;
    let operation = manager.get_operation(&project)?;

    Ok(TemplateState {
        project,
        workspace_name,
        workspace_present,
        snapshot_name,
        operation,
    })
}

#[tauri::command]
pub async fn templates_create_template(
    project: String,
    manager: State<'_, TemplateOperationManager>,
) -> Result<TemplateWorkspace, String> {
    manager.begin_operation(
        &project,
        TemplateOperationKind::Create,
        "ensuring_template_workspace",
        Some("Creating template workspace"),
    )?;

    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace = match ensure_template_workspace_for_operation(
        &project,
        TemplateOperationKind::Create,
    )
    .await
    {
        Ok(workspace) => workspace,
        Err(error) => {
            let _ = manager.fail_operation(&project, "ensuring_template_workspace", &error);
            return Err(error);
        }
    };

    manager.update_phase(
        &project,
        "waiting_for_template_ready",
        Some("Waiting for template workspace bootstrap"),
    )?;
    bootstrap::start_template_bootstrap(workspace_name);
    manager.start_reconcile(&project);

    Ok(workspace)
}

#[tauri::command]
pub async fn templates_edit_template(
    project: String,
    manager: State<'_, TemplateOperationManager>,
) -> Result<TemplateWorkspace, String> {
    manager.begin_operation(
        &project,
        TemplateOperationKind::Edit,
        "ensuring_template_workspace",
        Some("Creating template workspace from snapshot"),
    )?;

    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace = match ensure_template_workspace_for_operation(
        &project,
        TemplateOperationKind::Edit,
    )
    .await
    {
        Ok(workspace) => workspace,
        Err(error) => {
            let _ = manager.fail_operation(&project, "ensuring_template_workspace", &error);
            return Err(error);
        }
    };

    manager.update_phase(
        &project,
        "waiting_for_template_ready",
        Some("Waiting for template workspace bootstrap"),
    )?;
    bootstrap::start_template_bootstrap(workspace_name);
    manager.start_reconcile(&project);

    Ok(workspace)
}

#[tauri::command]
pub async fn templates_save_template(
    project: String,
    manager: State<'_, TemplateOperationManager>,
) -> Result<TemplateOperation, String> {
    let operation = manager.begin_operation(
        &project,
        TemplateOperationKind::Save,
        "waiting_for_template_ready",
        Some("Waiting for template workspace bootstrap"),
    )?;
    manager.start_reconcile(&project);
    Ok(operation)
}

#[tauri::command]
pub async fn templates_delete_template(
    project: String,
    manager: State<'_, TemplateOperationManager>,
) -> Result<TemplateOperation, String> {
    let operation = manager.begin_operation(
        &project,
        TemplateOperationKind::Delete,
        "deleting_template_workspace",
        Some("Deleting template workspace"),
    )?;
    manager.start_reconcile(&project);
    Ok(operation)
}

async fn reconcile_template_operation(
    manager: TemplateOperationManager,
    project: &str,
) -> Result<(), String> {
    let operation = manager
        .get_operation(project)?
        .ok_or_else(|| format!("template operation missing for project {project}"))?;
    if operation.status != TemplateOperationStatus::Running {
        return Ok(());
    }

    match operation.kind {
        TemplateOperationKind::Create | TemplateOperationKind::Edit => {
            reconcile_template_prepare_operation(manager, project, operation.kind).await
        }
        TemplateOperationKind::Save => reconcile_template_save_operation(manager, project).await,
        TemplateOperationKind::Delete => {
            reconcile_template_delete_operation(manager, project).await
        }
    }
}

async fn reconcile_template_prepare_operation(
    manager: TemplateOperationManager,
    project: &str,
    kind: TemplateOperationKind,
) -> Result<(), String> {
    manager.update_phase(
        project,
        "ensuring_template_workspace",
        Some(match kind {
            TemplateOperationKind::Create => "Creating template workspace",
            TemplateOperationKind::Edit => "Creating template workspace from snapshot",
            TemplateOperationKind::Save | TemplateOperationKind::Delete => {
                "Preparing template workspace"
            }
        }),
    )?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let _workspace = ensure_template_workspace_for_operation(project, kind).await?;
    bootstrap::start_template_bootstrap(workspace_name.clone());
    manager.update_phase(
        project,
        "waiting_for_template_ready",
        Some("Waiting for template workspace bootstrap"),
    )?;
    bootstrap::wait_for_template_bootstrap(&workspace_name).await?;
    manager.complete_operation(project, "ready_for_edit", Some("Template workspace ready"))?;
    Ok(())
}

async fn reconcile_template_save_operation(
    manager: TemplateOperationManager,
    project: &str,
) -> Result<(), String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    let phase = manager
        .get_operation(project)?
        .map(|operation| operation.phase)
        .unwrap_or_else(|| "waiting_for_template_ready".to_string());

    if save_phase_rank(&phase) <= save_phase_rank("clearing_runtime_state") {
        manager.update_phase(
            project,
            "waiting_for_template_ready",
            Some("Waiting for template workspace bootstrap"),
        )?;
        bootstrap::wait_for_template_bootstrap(&workspace_name).await?;
        manager.update_phase(
            project,
            "clearing_runtime_state",
            Some("Removing template-only runtime state"),
        )?;
        bootstrap::clear_template_runtime_state(&workspace_name).await?;
    }

    let phase = manager
        .get_operation(project)?
        .map(|operation| operation.phase)
        .unwrap_or_else(|| "stopping_vm".to_string());
    let mut boot_disk = None;
    if save_phase_rank(&phase) <= save_phase_rank("deleting_old_snapshots") {
        manager.update_phase(
            project,
            "stopping_vm",
            Some("Stopping template virtual machine"),
        )?;
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

    let mut operation = manager
        .get_operation(project)?
        .ok_or_else(|| format!("template operation missing for project {project}"))?;
    if operation.snapshot_name.is_none() {
        let boot_disk = boot_disk.ok_or_else(|| {
            format!("template save for project {project} is missing the template boot disk")
        })?;
        manager.update_phase(
            project,
            "creating_snapshot",
            Some("Creating template snapshot"),
        )?;
        let snapshot_name = workspaces::create_template_snapshot_for_disk(
            &gcloud.account,
            &gcloud.project,
            project,
            &boot_disk,
            &gcloud.zone,
        )
        .await?;
        manager.update_snapshot(project, &snapshot_name)?;
        operation = manager
            .get_operation(project)?
            .ok_or_else(|| format!("template operation missing for project {project}"))?;
    }

    manager.update_phase(
        project,
        "waiting_for_snapshot_ready",
        Some("Waiting for template snapshot"),
    )?;
    workspaces::wait_for_template_snapshot_ready(
        &gcloud.account,
        &gcloud.project,
        operation
            .snapshot_name
            .as_deref()
            .ok_or_else(|| format!("template snapshot missing for project {project}"))?,
    )
    .await?;

    manager.update_phase(
        project,
        "deleting_old_snapshots",
        Some("Removing previous template snapshots"),
    )?;
    workspaces::delete_old_template_snapshots(
        &gcloud.account,
        &gcloud.project,
        project,
        operation
            .snapshot_name
            .as_deref()
            .ok_or_else(|| format!("template snapshot missing for project {project}"))?,
    )
    .await?;

    manager.update_phase(
        project,
        "deleting_template_workspace",
        Some("Deleting template workspace"),
    )?;
    workspaces::delete_template_workspace_if_exists(
        &gcloud.account,
        &gcloud.project,
        &workspace_name,
        &gcloud.zone,
    )
    .await?;

    manager.complete_operation(project, "completed", Some("Template saved"))?;
    Ok(())
}

async fn reconcile_template_delete_operation(
    manager: TemplateOperationManager,
    project: &str,
) -> Result<(), String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);

    manager.update_phase(
        project,
        "deleting_template_workspace",
        Some("Deleting template workspace"),
    )?;
    workspaces::delete_template_workspace_if_exists(
        &gcloud.account,
        &gcloud.project,
        &workspace_name,
        &gcloud.zone,
    )
    .await?;

    manager.update_phase(
        project,
        "deleting_snapshots",
        Some("Deleting template snapshots"),
    )?;
    workspaces::delete_template_snapshots(&gcloud.account, &gcloud.project, project).await?;

    manager.complete_operation(project, "completed", Some("Template deleted"))?;
    Ok(())
}

async fn ensure_template_workspace_for_operation(
    project: &str,
    kind: TemplateOperationKind,
) -> Result<TemplateWorkspace, String> {
    let gcloud = resolve_project_gcloud_config(project)?;
    let workspace_name = workspaces::generate_template_workspace_name(project);
    if let Some(existing) =
        workspaces::find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project)
            .await?
    {
        return match existing {
            Workspace::Template(workspace) => Ok(workspace),
            Workspace::Branch(_) => Err(format!(
                "workspace name is already in use for project {project}: {workspace_name}"
            )),
        };
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

fn resolve_project_gcloud_config(project: &str) -> Result<ResolvedGcloudConfig, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    workspaces::resolve_project_gcloud_config(&config, project)
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

fn load_operations(path: &PathBuf) -> Result<HashMap<String, TemplateOperation>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let contents = fs::read_to_string(path)
        .map_err(|error| format!("failed to read template state {}: {error}", path.display()))?;
    let state: TemplateOperationFile = serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse template state {}: {error}", path.display()))?;
    Ok(state.operations)
}

fn persist_operations(
    path: &PathBuf,
    operations: &HashMap<String, TemplateOperation>,
) -> Result<(), String> {
    let state = TemplateOperationFile {
        operations: operations.clone(),
    };
    let contents = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("failed to serialize template state: {error}"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create template state directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temp_path = path.with_extension(format!("tmp-{}-{nanos}", std::process::id()));
    fs::write(&temp_path, contents).map_err(|error| {
        format!(
            "failed to write template state temp file {}: {error}",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, path).map_err(|error| {
        format!(
            "failed to replace template state file {}: {error}",
            path.display()
        )
    })?;
    Ok(())
}
