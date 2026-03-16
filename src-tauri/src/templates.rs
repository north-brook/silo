use crate::config::ConfigStore;
use crate::terminal;
use crate::workspaces::{self, SnapshotTemplate, TemplateWorkspace, Workspace};

#[tauri::command]
pub async fn templates_list_templates() -> Result<Vec<SnapshotTemplate>, String> {
    workspaces::list_template_snapshots().await
}

#[tauri::command]
pub async fn templates_create_template(project: String) -> Result<TemplateWorkspace, String> {
    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace = workspaces::create_template_workspace_for_project(&project, None).await?;
    terminal::start_template_bootstrap(workspace_name);
    Ok(workspace)
}

#[tauri::command]
pub async fn templates_edit_template(project: String) -> Result<TemplateWorkspace, String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let gcloud = workspaces::resolve_project_gcloud_config(&config, &project)?;
    let snapshot_name =
        workspaces::latest_template_snapshot_name(&gcloud.account, &gcloud.project, &project)
            .await?
            .ok_or_else(|| format!("template not found for project {project}"))?;

    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace =
        workspaces::create_template_workspace_for_project(&project, Some(snapshot_name)).await?;
    terminal::start_template_bootstrap(workspace_name);
    Ok(workspace)
}

#[tauri::command]
pub async fn templates_save_template(project: String) -> Result<(), String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let gcloud = workspaces::resolve_project_gcloud_config(&config, &project)?;
    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace =
        workspaces::find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project)
            .await?
            .ok_or_else(|| format!("template workspace not found for project {project}"))?;

    if !matches!(workspace, Workspace::Template(_)) {
        return Err(format!(
            "workspace name is already in use for project {project}: {workspace_name}"
        ));
    }

    terminal::wait_for_template_bootstrap(&workspace_name).await?;
    terminal::clear_template_runtime_state(&workspace_name).await?;

    let zone = workspace.zone().to_string();
    workspaces::stop_and_snapshot_template_workspace(
        gcloud.account.clone(),
        gcloud.project.clone(),
        project.clone(),
        workspace_name.clone(),
        zone.clone(),
    )
    .await?;

    let delete_result = workspaces::run_gcloud(
        &gcloud.account,
        &gcloud.project,
        workspaces::delete_workspace_args(&workspace_name, &zone),
    )
    .await?;
    if !delete_result.success {
        return Err(workspaces::gcloud_error(
            "failed to delete template workspace",
            &delete_result.stderr,
        ));
    }

    Ok(())
}

#[tauri::command]
pub async fn templates_delete_template(project: String) -> Result<(), String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let gcloud = workspaces::resolve_project_gcloud_config(&config, &project)?;
    let workspace_name = workspaces::generate_template_workspace_name(&project);
    let workspace =
        workspaces::find_workspace_in_project(&workspace_name, &gcloud.account, &gcloud.project)
            .await?;

    if let Some(workspace) = workspace {
        if !matches!(workspace, Workspace::Template(_)) {
            return Err(format!(
                "workspace name is already in use for project {project}: {workspace_name}"
            ));
        }

        let delete_result = workspaces::run_gcloud(
            &gcloud.account,
            &gcloud.project,
            workspaces::delete_workspace_args(&workspace_name, workspace.zone()),
        )
        .await?;
        if !delete_result.success {
            return Err(workspaces::gcloud_error(
                "failed to delete template workspace",
                &delete_result.stderr,
            ));
        }
    }

    let snapshots =
        workspaces::list_template_snapshots_in_project(&gcloud.account, &gcloud.project, &project)
            .await?;

    for snapshot in snapshots {
        let delete_result = workspaces::run_gcloud(
            &gcloud.account,
            &gcloud.project,
            workspaces::delete_snapshot_args(&snapshot.name),
        )
        .await?;
        if !delete_result.success {
            return Err(workspaces::gcloud_error(
                "failed to delete template snapshot",
                &delete_result.stderr,
            ));
        }
    }

    Ok(())
}
