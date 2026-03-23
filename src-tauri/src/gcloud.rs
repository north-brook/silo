use crate::config::ConfigStore;
use crate::gcp;
use crate::state_paths;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const SERVICE_ACCOUNT_ID: &str = "silo-workspaces";
const SERVICE_ACCOUNT_DISPLAY_NAME: &str = "Silo Workspaces";
const PROJECT_IAM_ROLES: &[&str] = &[
    "roles/compute.instanceAdmin.v1",
    "roles/compute.osAdminLogin",
];

struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn run_gcloud<I, S>(args: I) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let command_line = args.join(" ");

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let output = Command::new("gcloud").args(&args).output().ok()?;
        if output.status.success() {
            log::trace!(
                "gcloud command completed duration_ms={} args={command_line}",
                started.elapsed().as_millis()
            );
        } else {
            log::warn!(
                "gcloud command failed duration_ms={} args={} stderr={}",
                started.elapsed().as_millis(),
                command_line,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Some(CommandResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    })
    .await
    .ok()
    .flatten()
}

async fn run_gcloud_as<I, S>(account: &str, args: I) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut prefixed = Vec::new();
    let account = account.trim();
    if !account.is_empty() {
        prefixed.push(format!("--account={account}"));
    }
    prefixed.extend(args.into_iter().map(Into::into));
    run_gcloud(prefixed).await
}

fn build_auth_login_args() -> [&'static str; 3] {
    ["auth", "login", "--update-adc"]
}

fn has_service_account_identity(config: &crate::config::GcloudConfig) -> bool {
    !config.service_account.trim().is_empty()
        && !config.service_account_key_file.trim().is_empty()
        && !config.project.trim().is_empty()
}

fn has_local_gcloud_identity(config: &crate::config::GcloudConfig) -> bool {
    has_service_account_identity(config) && Path::new(&config.service_account_key_file).is_file()
}

fn configured_service_account(config: &crate::config::GcloudConfig) -> Option<&str> {
    let service_account = config.service_account.trim();
    if service_account.is_empty() {
        None
    } else {
        Some(service_account)
    }
}

fn service_account_email(project: &str) -> String {
    format!("{SERVICE_ACCOUNT_ID}@{project}.iam.gserviceaccount.com")
}

fn service_account_key_path(project: &str) -> Result<PathBuf, String> {
    let parent = state_paths::app_state_dir()?;
    let safe_project = project
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();

    Ok(parent.join(format!("{safe_project}-{SERVICE_ACCOUNT_ID}.json")))
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
    }

    Ok(())
}

async fn gcloud_service_account_usable(service_account: &str, project: &str) -> Result<(), String> {
    if service_account.trim().is_empty() || project.trim().is_empty() {
        return Err("gcloud service account or project is unset".to_string());
    }

    let token_result = run_gcloud([
        "auth".to_string(),
        "print-access-token".to_string(),
        service_account.to_string(),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud auth print-access-token".to_string())?;

    if !token_result.success || token_result.stdout.trim().is_empty() {
        let stderr = token_result.stderr.trim();
        return Err(if stderr.is_empty() {
            "gcloud could not mint an access token non-interactively".to_string()
        } else {
            stderr.to_string()
        });
    }

    let compute_result = run_gcloud([
        "compute".to_string(),
        "instances".to_string(),
        "list".to_string(),
        "--account".to_string(),
        service_account.to_string(),
        "--project".to_string(),
        project.to_string(),
        "--limit=1".to_string(),
        "--format=value(name)".to_string(),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud compute instances list".to_string())?;

    if !compute_result.success {
        let stderr = compute_result.stderr.trim();
        return Err(if stderr.is_empty() {
            format!(
                "gcloud service account could not access compute instances in project {project}"
            )
        } else {
            stderr.to_string()
        });
    }

    Ok(())
}

async fn service_account_exists(service_account: &str, project: &str) -> Result<bool, String> {
    let result = run_gcloud([
        "iam".to_string(),
        "service-accounts".to_string(),
        "describe".to_string(),
        service_account.to_string(),
        "--project".to_string(),
        project.to_string(),
        "--format=value(email)".to_string(),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud iam service-accounts describe".to_string())?;

    Ok(result.success)
}

async fn ensure_service_account(project: &str) -> Result<String, String> {
    let service_account = service_account_email(project);
    if service_account_exists(&service_account, project).await? {
        return Ok(service_account);
    }

    let result = run_gcloud([
        "iam".to_string(),
        "service-accounts".to_string(),
        "create".to_string(),
        SERVICE_ACCOUNT_ID.to_string(),
        "--project".to_string(),
        project.to_string(),
        format!("--display-name={SERVICE_ACCOUNT_DISPLAY_NAME}"),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud iam service-accounts create".to_string())?;

    if !result.success {
        let stderr = result.stderr.trim();
        return Err(if stderr.is_empty() {
            "failed to create service account".to_string()
        } else {
            stderr.to_string()
        });
    }

    Ok(service_account)
}

async fn ensure_project_iam_bindings(project: &str, service_account: &str) -> Result<(), String> {
    ensure_project_iam_bindings_as("", project, service_account).await
}

async fn ensure_project_iam_bindings_as(
    operator_account: &str,
    project: &str,
    service_account: &str,
) -> Result<(), String> {
    for role in PROJECT_IAM_ROLES {
        let result = run_gcloud_as(
            operator_account,
            [
            "projects".to_string(),
            "add-iam-policy-binding".to_string(),
            project.to_string(),
            "--member".to_string(),
            format!("serviceAccount:{service_account}"),
            "--role".to_string(),
            (*role).to_string(),
            "--quiet".to_string(),
        ],
        )
        .await
        .ok_or_else(|| "failed to execute gcloud projects add-iam-policy-binding".to_string())?;

        if !result.success {
            let stderr = result.stderr.trim();
            return Err(if stderr.is_empty() {
                format!("failed to grant service account project role {role}")
            } else {
                stderr.to_string()
            });
        }
    }

    Ok(())
}

async fn ensure_service_account_user_binding(
    project: &str,
    service_account: &str,
) -> Result<(), String> {
    ensure_service_account_user_binding_as("", project, service_account).await
}

async fn ensure_service_account_user_binding_as(
    operator_account: &str,
    project: &str,
    service_account: &str,
) -> Result<(), String> {
    let result = run_gcloud_as(
        operator_account,
        [
        "iam".to_string(),
        "service-accounts".to_string(),
        "add-iam-policy-binding".to_string(),
        service_account.to_string(),
        "--project".to_string(),
        project.to_string(),
        "--member".to_string(),
        format!("serviceAccount:{service_account}"),
        "--role".to_string(),
        "roles/iam.serviceAccountUser".to_string(),
        "--quiet".to_string(),
    ],
    )
    .await
    .ok_or_else(|| {
        "failed to execute gcloud iam service-accounts add-iam-policy-binding".to_string()
    })?;

    if !result.success {
        let stderr = result.stderr.trim();
        return Err(if stderr.is_empty() {
            "failed to grant service account user permissions".to_string()
        } else {
            stderr.to_string()
        });
    }

    Ok(())
}

async fn activate_service_account(
    service_account: &str,
    key_path: &Path,
    project: &str,
) -> Result<(), String> {
    let result = run_gcloud([
        "auth".to_string(),
        "activate-service-account".to_string(),
        service_account.to_string(),
        format!("--key-file={}", key_path.display()),
        "--project".to_string(),
        project.to_string(),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud auth activate-service-account".to_string())?;

    if !result.success {
        let stderr = result.stderr.trim();
        return Err(if stderr.is_empty() {
            "failed to activate service account".to_string()
        } else {
            stderr.to_string()
        });
    }

    Ok(())
}

async fn ensure_service_account_key(
    service_account: &str,
    project: &str,
) -> Result<PathBuf, String> {
    let key_path = service_account_key_path(project)?;
    let parent = key_path
        .parent()
        .ok_or_else(|| format!("invalid service account key path: {}", key_path.display()))?;
    ensure_private_dir(parent)?;

    if key_path.is_file()
        && activate_service_account(service_account, &key_path, project)
            .await
            .is_ok()
    {
        return Ok(key_path);
    }

    if key_path.exists() {
        fs::remove_file(&key_path).map_err(|error| {
            format!(
                "failed to remove stale key file {}: {error}",
                key_path.display()
            )
        })?;
    }

    let result = run_gcloud([
        "iam".to_string(),
        "service-accounts".to_string(),
        "keys".to_string(),
        "create".to_string(),
        key_path.display().to_string(),
        "--iam-account".to_string(),
        service_account.to_string(),
        "--project".to_string(),
        project.to_string(),
        "--quiet".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gcloud iam service-accounts keys create".to_string())?;

    if !result.success {
        let stderr = result.stderr.trim();
        return Err(if stderr.is_empty() {
            "failed to create service account key".to_string()
        } else {
            stderr.to_string()
        });
    }

    Ok(key_path)
}

fn save_gcloud_identity(
    account: &str,
    project: &str,
    service_account: &str,
    service_account_key_file: &str,
) -> Result<(), String> {
    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    let mut config = store.load().map_err(|error| error.to_string())?;
    config.gcloud.account = account.to_string();
    config.gcloud.project = project.to_string();
    config.gcloud.service_account = service_account.to_string();
    config.gcloud.service_account_key_file = service_account_key_file.to_string();
    store.save(&config).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn gcloud_installed() -> bool {
    log::trace!("checking whether gcloud is installed");
    run_gcloud(["version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gcloud_authenticate() -> Result<(), String> {
    log::info!("starting gcloud authentication");
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("gcloud")
            .args(build_auth_login_args())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| format!("failed to start gcloud auth login: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("gcloud auth login exited with status {status}"))
                }
            })
    })
    .await
    .map_err(|error| format!("gcloud auth login task failed: {error}"))??;

    let account = run_gcloud(["config", "get-value", "account"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|value| !value.is_empty() && value != "(unset)")
        .unwrap_or_default();
    let project = run_gcloud(["config", "get-value", "project"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|value| !value.is_empty() && value != "(unset)")
        .unwrap_or_default();

    if account.is_empty() || project.is_empty() {
        return Err("gcloud auth completed but account or project is still unset".to_string());
    }
    log::info!("gcloud auth completed for account {account} project {project}");

    let service_account = ensure_service_account(&project).await?;
    ensure_project_iam_bindings(&project, &service_account).await?;
    ensure_service_account_user_binding(&project, &service_account).await?;
    let key_path = ensure_service_account_key(&service_account, &project).await?;
    let key_path_string = key_path.to_string_lossy().into_owned();
    save_gcloud_identity(&account, &project, &service_account, &key_path_string)?;
    log::info!("gcloud service account {service_account} saved for project {project}");

    activate_service_account(&service_account, &key_path, &project)
        .await
        .map_err(|error| {
            format!("service account key was provisioned and saved, but activation failed: {error}")
        })?;

    gcp::ensure_runtime_oslogin_ready(&project)
        .await
        .map_err(|error| {
            format!(
                "service account was provisioned and activated, but OS Login is not ready yet: {error}"
            )
        })?;

    gcloud_service_account_usable(&service_account, &project)
        .await
        .map_err(|error| {
            format!("service account was provisioned and saved, but it is not usable yet: {error}")
        })
}

pub(crate) async fn repair_runtime_identity_if_needed() -> Result<(), String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    if !has_local_gcloud_identity(&config.gcloud) {
        return Ok(());
    }

    let project = config.gcloud.project.trim().to_string();
    let service_account = config.gcloud.service_account.trim().to_string();
    if project.is_empty() || service_account.is_empty() {
        return Ok(());
    }

    match gcp::ensure_runtime_oslogin_ready(&project).await {
        Ok(username) => {
            log::info!(
                "runtime service account {} is OS Login ready as {}",
                service_account,
                username
            );
            return Ok(());
        }
        Err(error) => {
            log::warn!(
                "runtime service account {} needs OS Login repair in project {}: {}",
                service_account,
                project,
                error
            );
        }
    }

    let operator_account = config.gcloud.account.trim().to_string();
    if operator_account.is_empty() {
        return Err(
            "runtime service account needs OS Login repair but no operator gcloud account is configured"
                .to_string(),
        );
    }

    ensure_project_iam_bindings_as(&operator_account, &project, &service_account).await?;
    ensure_service_account_user_binding_as(&operator_account, &project, &service_account).await?;
    let username = gcp::ensure_runtime_oslogin_ready(&project).await?;
    log::info!(
        "runtime service account {} repaired for project {} as OS Login user {}",
        service_account,
        project,
        username
    );
    Ok(())
}

#[tauri::command]
pub fn gcloud_configure(account: String, project: String) -> Result<(), String> {
    log::info!("saving manual gcloud configuration for account {account} project {project}");
    save_gcloud_identity(&account, &project, "", "")?;
    Ok(())
}

#[tauri::command]
pub async fn gcloud_configured() -> bool {
    log::trace!("checking whether gcloud is configured");
    let Ok(config) = ConfigStore::new().and_then(|store| store.load()) else {
        return false;
    };

    has_local_gcloud_identity(&config.gcloud)
}

#[tauri::command]
pub async fn gcloud_accounts() -> Vec<String> {
    log::trace!("listing gcloud accounts");
    if let Ok(config) = ConfigStore::new().and_then(|store| store.load()) {
        if let Some(service_account) = configured_service_account(&config.gcloud) {
            return vec![service_account.to_string()];
        }
    }

    run_gcloud([
        "auth",
        "list",
        "--filter=status:ACTIVE",
        "--format=value(account)",
    ])
    .await
    .filter(|result| result.success)
    .map(|result| {
        result
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

#[tauri::command]
pub async fn gcloud_projects(account: String) -> Vec<String> {
    log::trace!("listing gcloud projects");
    let configured_service_account = ConfigStore::new()
        .and_then(|store| store.load())
        .ok()
        .and_then(|config| configured_service_account(&config.gcloud).map(str::to_owned));
    let account = configured_service_account.unwrap_or(account);

    let Some(result) = run_gcloud([
        "projects".to_string(),
        "list".to_string(),
        "--account".to_string(),
        account,
        "--format=value(projectId)".to_string(),
    ])
    .await
    else {
        return Vec::new();
    };

    if !result.success {
        return Vec::new();
    }

    result
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigStore, GcloudConfig};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn auth_login_updates_adc() {
        assert_eq!(build_auth_login_args(), ["auth", "login", "--update-adc"]);
    }

    #[test]
    fn has_service_account_identity_requires_service_account_key_and_project() {
        let mut config = GcloudConfig::default();
        assert!(!has_service_account_identity(&config));

        config.service_account = "svc@example.iam.gserviceaccount.com".to_string();
        assert!(!has_service_account_identity(&config));

        config.service_account_key_file =
            "/Users/test/.silo/demo-project-silo-workspaces.json".to_string();
        assert!(!has_service_account_identity(&config));

        config.project = "demo-project".to_string();
        assert!(has_service_account_identity(&config));
    }

    #[test]
    fn has_local_gcloud_identity_requires_key_file_to_exist() {
        let temp_home = std::env::temp_dir().join(format!(
            "silo-gcloud-local-identity-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_home).unwrap();
        let key_path = temp_home.join("demo-project-silo-workspaces.json");

        let mut config = GcloudConfig {
            service_account: "svc@demo-project.iam.gserviceaccount.com".to_string(),
            service_account_key_file: key_path.to_string_lossy().into_owned(),
            project: "demo-project".to_string(),
            ..GcloudConfig::default()
        };

        assert!(!has_local_gcloud_identity(&config));

        fs::write(&key_path, "{\"type\":\"service_account\"}").unwrap();
        assert!(has_local_gcloud_identity(&config));

        config.service_account.clear();
        assert!(!has_local_gcloud_identity(&config));

        let _ = fs::remove_dir_all(temp_home);
    }

    #[test]
    fn service_account_email_uses_project_domain() {
        assert_eq!(
            service_account_email("demo-project"),
            "silo-workspaces@demo-project.iam.gserviceaccount.com"
        );
    }

    #[test]
    fn project_iam_roles_include_os_admin_login() {
        assert!(PROJECT_IAM_ROLES.contains(&"roles/compute.instanceAdmin.v1"));
        assert!(PROJECT_IAM_ROLES.contains(&"roles/compute.osAdminLogin"));
    }

    #[test]
    fn save_gcloud_identity_updates_all_fields_together() {
        let temp_home = std::env::temp_dir().join(format!(
            "silo-gcloud-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_home).unwrap();

        let store = ConfigStore::from_home_dir(temp_home.clone());
        store.initialize_defaults_if_missing().unwrap();

        let original_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &temp_home);

        save_gcloud_identity(
            "user@example.com",
            "demo-project",
            "svc@demo-project.iam.gserviceaccount.com",
            "/Users/test/.silo/demo-project-silo-workspaces.json",
        )
        .unwrap();

        let config = store.load().unwrap();
        assert_eq!(config.gcloud.account, "user@example.com");
        assert_eq!(config.gcloud.project, "demo-project");
        assert_eq!(
            config.gcloud.service_account,
            "svc@demo-project.iam.gserviceaccount.com"
        );
        assert_eq!(
            config.gcloud.service_account_key_file,
            "/Users/test/.silo/demo-project-silo-workspaces.json"
        );

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }

        let _ = fs::remove_dir_all(temp_home);
    }
}
