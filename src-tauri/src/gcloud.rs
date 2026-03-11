use crate::config::{set_gcloud_config, ConfigStore};
use std::process::{Command, Stdio};

struct CommandResult {
    success: bool,
    stdout: String,
}

async fn run_gcloud<I, S>(args: I) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();

    tauri::async_runtime::spawn_blocking(move || {
        let output = Command::new("gcloud").args(&args).output().ok()?;

        Some(CommandResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn gcloud_installed() -> bool {
    run_gcloud(["version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gcloud_authenticate() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("gcloud")
            .args(["auth", "login"])
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

    gcloud_configure(account, project)
}

#[tauri::command]
pub fn gcloud_configure(account: String, project: String) -> Result<(), String> {
    set_gcloud_config(account, project).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn gcloud_configured() -> bool {
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| {
            !config.gcloud.account.trim().is_empty() && !config.gcloud.project.trim().is_empty()
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gcloud_accounts() -> Vec<String> {
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
