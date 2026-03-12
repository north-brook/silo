use crate::config::{ConfigStore, ProjectConfig, SiloConfig};
use crate::river_names::DEFAULT_RIVER_NAMES;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Workspace {
    name: String,
    project: Option<String>,
    branch: String,
    target_branch: String,
    unread: bool,
    working: Option<bool>,
    last_active: Option<String>,
    created_at: String,
    status: String,
    zone: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedGcloudConfig {
    account: String,
    service_account: String,
    project: String,
    region: String,
    zone: String,
    machine_type: String,
    disk_size_gb: u32,
    disk_type: String,
    image_family: String,
    image_project: String,
}

#[derive(Debug)]
struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone)]
struct WorkspaceLookup {
    workspace: Workspace,
    account: String,
    gcloud_project: String,
}

#[tauri::command]
pub async fn workspaces_list_workspaces() -> Result<Vec<Workspace>, String> {
    log::info!("listing workspaces across configured gcloud projects");
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
    log::info!("listed {} workspaces", workspaces.len());

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
    let workspace_name = generate_workspace_name(&project);
    let branch_name = default_workspace_branch();

    let result = run_gcloud(
        &gcloud.account,
        &gcloud.project,
        create_workspace_args(
            &workspace_name,
            &project,
            &branch_name,
            &project_config.target_branch,
            &gcloud,
        ),
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to create workspace", &result.stderr));
    }

    log::info!("workspace {workspace_name} creation started for project {project}");
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
            workspace,
            format!("--zone={}", lookup.workspace.zone),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to start workspace", &result.stderr));
    }

    log::info!("workspace {} started", lookup.workspace.name);
    Ok(())
}

#[tauri::command]
pub async fn workspaces_stop_workspace(workspace: String) -> Result<(), String> {
    log::info!("stopping workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        [
            "compute".to_string(),
            "instances".to_string(),
            "stop".to_string(),
            workspace,
            format!("--zone={}", lookup.workspace.zone),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to stop workspace", &result.stderr));
    }

    log::info!("workspace {} stopped", lookup.workspace.name);
    Ok(())
}

#[tauri::command]
pub async fn workspaces_get_workspace(workspace: String) -> Result<Workspace, String> {
    log::trace!("getting workspace {workspace}");
    Ok(find_workspace(&workspace).await?.workspace)
}

#[tauri::command]
pub async fn workspaces_delete_workspace(workspace: String) -> Result<(), String> {
    log::info!("deleting workspace {workspace}");
    let lookup = find_workspace(&workspace).await?;

    let result = run_gcloud(
        &lookup.account,
        &lookup.gcloud_project,
        [
            "compute".to_string(),
            "instances".to_string(),
            "delete".to_string(),
            workspace,
            format!("--zone={}", lookup.workspace.zone),
            "--quiet".to_string(),
        ],
    )
    .await?;

    if !result.success {
        return Err(gcloud_error("failed to delete workspace", &result.stderr));
    }

    log::info!("workspace {} deleted", lookup.workspace.name);
    Ok(())
}

#[tauri::command]
pub async fn workspaces_update_workspace_branch(
    workspace: String,
    branch: String,
) -> Result<(), String> {
    log::info!("updating branch label for workspace {workspace}");
    update_workspace_label(&workspace, "branch", &branch).await
}

#[tauri::command]
pub async fn workspaces_update_workspace_target_branch(
    workspace: String,
    target_branch: String,
) -> Result<(), String> {
    log::info!("updating target branch label for workspace {workspace}");
    update_workspace_label(&workspace, "target_branch", &target_branch).await
}

async fn find_workspace(name: &str) -> Result<WorkspaceLookup, String> {
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

async fn update_workspace_label(workspace: &str, label: &str, value: &str) -> Result<(), String> {
    let lookup = find_workspace(workspace).await?;
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
                label, lookup.workspace.name
            ),
            &result.stderr,
        ));
    }

    log::info!(
        "updated {} label for workspace {}",
        label,
        lookup.workspace.name
    );
    Ok(())
}

async fn find_workspace_in_project(
    name: &str,
    account: &str,
    project: &str,
) -> Result<Option<Workspace>, String> {
    let mut workspaces = list_workspaces_in_project(account, project)
        .await?
        .into_iter()
        .filter(|workspace| workspace.name == name)
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
            "--format=json(name,zone,status,labels,creationTimestamp)".to_string(),
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

    Workspace {
        name: name.to_string(),
        project: Some(sanitize_label_value(project_label)),
        branch: branch_label.to_string(),
        target_branch: target_branch.to_string(),
        unread: false,
        working: None,
        last_active: None,
        created_at,
        status: "PROVISIONING".to_string(),
        zone: zone.to_string(),
    }
}

async fn describe_workspace_in_project(
    name: &str,
    account: &str,
    project: &str,
) -> Result<Workspace, String> {
    find_workspace_in_project(name, account, project)
        .await?
        .ok_or_else(|| format!("workspace not found after creation: {name}"))
}

fn resolve_project_gcloud_config(
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

fn candidate_gcloud_configs(config: &SiloConfig) -> Vec<ResolvedGcloudConfig> {
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
    gcloud: &ResolvedGcloudConfig,
) -> Vec<String> {
    let mut labels = vec![
        format!("project={}", sanitize_label_value(project_label)),
        format!("branch={}", sanitize_label_value(branch_label)),
        "unread=false".to_string(),
    ];
    let sanitized_target_branch = sanitize_label_value(target_branch);
    if !sanitized_target_branch.is_empty() {
        labels.push(format!("target_branch={sanitized_target_branch}"));
    }

    let mut args = vec![
        "compute".to_string(),
        "instances".to_string(),
        "create".to_string(),
        workspace_name.to_string(),
        format!("--zone={}", gcloud.zone),
        format!("--machine-type={}", gcloud.machine_type),
        format!("--boot-disk-size={}GB", gcloud.disk_size_gb),
        format!("--boot-disk-type={}", gcloud.disk_type),
        format!("--image-family={}", gcloud.image_family),
        format!("--image-project={}", gcloud.image_project),
        format!("--labels={}", labels.join(",")),
        "--async".to_string(),
    ];

    if gcloud.service_account.trim().is_empty() {
        args.push("--no-service-account".to_string());
        args.push("--no-scopes".to_string());
    } else {
        args.push(format!("--service-account={}", gcloud.service_account));
        args.push("--scopes=https://www.googleapis.com/auth/compute".to_string());
    }

    args
}

fn update_workspace_label_args(workspace: &Workspace, label: &str, value: &str) -> Vec<String> {
    let sanitized_value = sanitize_label_value(value);
    if sanitized_value.is_empty() {
        vec![
            "compute".to_string(),
            "instances".to_string(),
            "remove-labels".to_string(),
            workspace.name.clone(),
            format!("--zone={}", workspace.zone),
            format!("--labels={label}"),
        ]
    } else {
        vec![
            "compute".to_string(),
            "instances".to_string(),
            "add-labels".to_string(),
            workspace.name.clone(),
            format!("--zone={}", workspace.zone),
            format!("--labels={label}={sanitized_value}"),
        ]
    }
}

fn default_workspace_branch() -> String {
    if DEFAULT_RIVER_NAMES.is_empty() {
        return "silo/workspace".to_string();
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!(
        "silo/{}",
        DEFAULT_RIVER_NAMES[nanos as usize % DEFAULT_RIVER_NAMES.len()]
    )
}

async fn run_gcloud<I, S>(account: &str, project: &str, args: I) -> Result<CommandResult, String>
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

fn parse_workspaces(stdout: &str) -> Result<Vec<Workspace>, String> {
    let value: Value =
        serde_json::from_str(stdout).map_err(|error| format!("invalid gcloud json: {error}"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| "gcloud did not return a JSON array".to_string())?;

    entries.iter().map(parse_workspace).collect()
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

    let project = labels
        .get("project")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let branch = labels
        .get("branch")
        .and_then(Value::as_str)
        .map(parse_branch_label)
        .unwrap_or_default();
    let target_branch = labels
        .get("target_branch")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    let unread = labels
        .get("unread")
        .and_then(Value::as_str)
        .map(|value| parse_bool_label("unread", value))
        .transpose()?
        .unwrap_or(false);
    let working = labels
        .get("working")
        .and_then(Value::as_str)
        .map(|value| parse_bool_label("working", value))
        .transpose()?;
    let last_active = labels
        .get("last_active")
        .and_then(Value::as_str)
        .map(str::to_owned);

    Ok(Workspace {
        name,
        project,
        branch,
        target_branch,
        unread,
        working,
        last_active,
        created_at,
        status,
        zone,
    })
}

fn parse_branch_label(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(river) = trimmed.strip_prefix("silo-") {
        if DEFAULT_RIVER_NAMES.contains(&river) {
            return format!("silo/{river}");
        }
    }

    trimmed.to_string()
}

fn required_string_field(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("workspace is missing {field}"))
}

fn zone_name(value: Option<&Value>) -> Option<String> {
    let zone = value?.as_str()?.trim();
    if zone.is_empty() {
        return None;
    }

    zone.rsplit('/').next().map(str::to_owned)
}

fn parse_bool_label(label: &str, value: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("invalid {label} label value: {other}")),
    }
}

fn compare_workspaces_by_last_active_desc(
    left: &Workspace,
    right: &Workspace,
) -> std::cmp::Ordering {
    match (&left.last_active, &right.last_active) {
        (Some(left_last_active), Some(right_last_active)) => right_last_active
            .cmp(left_last_active)
            .then_with(|| left.name.cmp(&right.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.name.cmp(&right.name),
    }
}

fn generate_workspace_name(project: &str) -> String {
    let normalized = normalize_instance_component(project);
    let suffix = unique_suffix();
    let max_project_len = 63usize.saturating_sub("ws--".len() + suffix.len());
    let truncated_project = truncate_to_boundary(&normalized, max_project_len);
    format!("ws-{truncated_project}-{suffix}")
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
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

fn gcloud_error(context: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GcloudConfig, ProjectGcloudConfig, DEFAULT_GCLOUD_DISK_SIZE_GB};
    use indexmap::IndexMap;
    use serde_json::json;

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
            gh: Default::default(),
            codex: Default::default(),
            claude: Default::default(),
            projects: IndexMap::new(),
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            target_branch: String::new(),
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
            gh: Default::default(),
            codex: Default::default(),
            claude: Default::default(),
            projects: IndexMap::new(),
        };
        let project = ProjectConfig {
            name: "demo".to_string(),
            path: "/tmp/demo".to_string(),
            image: None,
            target_branch: String::new(),
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
            &gcloud,
        );

        assert!(args.contains(&"--zone=us-east4-c".to_string()));
        assert!(args.contains(&"--machine-type=e2-standard-4".to_string()));
        assert!(args.contains(&"--boot-disk-size=80GB".to_string()));
        assert!(args.contains(&"--boot-disk-type=pd-ssd".to_string()));
        assert!(args.contains(&"--image-family=ubuntu-2404-lts-amd64".to_string()));
        assert!(args.contains(&"--image-project=ubuntu-os-cloud".to_string()));
        assert!(args.contains(&"--async".to_string()));
        assert!(args.contains(
            &"--service-account=silo-workspaces@proj.iam.gserviceaccount.com".to_string()
        ));
        assert!(args.contains(&"--scopes=https://www.googleapis.com/auth/compute".to_string()));
        assert!(args.contains(
            &"--labels=project=demo-project,branch=aare,unread=false,target_branch=feature-inbox"
                .to_string()
        ));
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

        let args = create_workspace_args("ws-demo-abc", "Demo Project", "Aare", "", &gcloud);

        assert!(args.contains(&"--async".to_string()));
        assert!(args.contains(&"--no-service-account".to_string()));
        assert!(args.contains(&"--no-scopes".to_string()));
        assert!(
            args.contains(&"--labels=project=demo-project,branch=aare,unread=false".to_string())
        );
    }

    #[test]
    fn default_workspace_branch_uses_ported_river_names() {
        let branch = default_workspace_branch();

        let river = branch
            .strip_prefix("silo/")
            .expect("default branch should use silo/ prefix");
        assert!(DEFAULT_RIVER_NAMES.contains(&river));
    }

    #[test]
    fn update_workspace_label_args_adds_label_when_value_present() {
        let args = update_workspace_label_args(
            &Workspace {
                name: "ws-demo-123".to_string(),
                project: Some("demo".to_string()),
                branch: "silo/aare".to_string(),
                target_branch: String::new(),
                unread: false,
                working: None,
                last_active: None,
                created_at: "2026-03-11T10:00:00Z".to_string(),
                status: "RUNNING".to_string(),
                zone: "us-east1-b".to_string(),
            },
            "target_branch",
            "Feature/Inbox",
        );

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "add-labels".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--labels=target_branch=feature-inbox".to_string(),
            ]
        );
    }

    #[test]
    fn update_workspace_label_args_removes_label_when_value_empty() {
        let args = update_workspace_label_args(
            &Workspace {
                name: "ws-demo-123".to_string(),
                project: Some("demo".to_string()),
                branch: "silo/aare".to_string(),
                target_branch: String::new(),
                unread: false,
                working: None,
                last_active: None,
                created_at: "2026-03-11T10:00:00Z".to_string(),
                status: "RUNNING".to_string(),
                zone: "us-east1-b".to_string(),
            },
            "branch",
            "",
        );

        assert_eq!(
            args,
            vec![
                "compute".to_string(),
                "instances".to_string(),
                "remove-labels".to_string(),
                "ws-demo-123".to_string(),
                "--zone=us-east1-b".to_string(),
                "--labels=branch".to_string(),
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
                "branch": "silo-aare",
                "target_branch": "main",
                "unread": "true",
                "working": "true",
                "last_active": "2026-03-11T13:05:00Z"
            }
        }))
        .expect("workspace should parse");

        assert_eq!(workspace.name, "ws-demo-123");
        assert_eq!(workspace.project.as_deref(), Some("demo"));
        assert_eq!(workspace.branch, "silo/aare");
        assert_eq!(workspace.target_branch, "main");
        assert!(workspace.unread);
        assert_eq!(workspace.working, Some(true));
        assert_eq!(
            workspace.last_active.as_deref(),
            Some("2026-03-11T13:05:00Z")
        );
        assert_eq!(workspace.created_at, "2026-03-11T13:00:00.000-04:00");
        assert_eq!(workspace.zone, "us-east1-b");
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

        assert_eq!(workspace.project, None);
        assert_eq!(workspace.branch, "");
        assert_eq!(workspace.target_branch, "");
        assert!(!workspace.unread);
        assert_eq!(workspace.working, None);
        assert_eq!(workspace.last_active, None);
    }

    #[test]
    fn compare_workspaces_sorts_last_active_desc_with_nulls_last() {
        let mut workspaces = vec![
            Workspace {
                name: "c".to_string(),
                project: Some("demo".to_string()),
                branch: String::new(),
                target_branch: String::new(),
                unread: false,
                working: None,
                last_active: None,
                created_at: "2026-03-11T10:00:00Z".to_string(),
                status: "RUNNING".to_string(),
                zone: "us-east1-b".to_string(),
            },
            Workspace {
                name: "b".to_string(),
                project: Some("demo".to_string()),
                branch: String::new(),
                target_branch: String::new(),
                unread: false,
                working: None,
                last_active: Some("2026-03-11T11:00:00Z".to_string()),
                created_at: "2026-03-11T10:00:00Z".to_string(),
                status: "RUNNING".to_string(),
                zone: "us-east1-b".to_string(),
            },
            Workspace {
                name: "a".to_string(),
                project: Some("demo".to_string()),
                branch: String::new(),
                target_branch: String::new(),
                unread: false,
                working: None,
                last_active: Some("2026-03-11T12:00:00Z".to_string()),
                created_at: "2026-03-11T10:00:00Z".to_string(),
                status: "RUNNING".to_string(),
                zone: "us-east1-b".to_string(),
            },
        ];

        workspaces.sort_by(compare_workspaces_by_last_active_desc);

        assert_eq!(
            workspaces
                .iter()
                .map(|workspace| workspace.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn generate_workspace_name_normalizes_and_truncates() {
        let name = generate_workspace_name("123 Very Loud Project Name With Spaces And Symbols!!!");

        assert!(name.starts_with("ws-w123-very-loud-project-name-with-spaces"));
        assert!(name.len() <= 63);
        assert!(name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'));
    }
}
