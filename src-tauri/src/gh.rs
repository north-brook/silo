use crate::config::ConfigStore;
use std::process::Command;

struct CommandResult {
    success: bool,
    stdout: String,
}

async fn run_gh<I, S>(args: I) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();

    tauri::async_runtime::spawn_blocking(move || {
        let output = Command::new("gh").args(&args).output().ok()?;

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
pub async fn gh_installed() -> bool {
    run_gh(["--version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_configured() -> bool {
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.gh.username.trim().is_empty() && !config.gh.token.trim().is_empty())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_username() -> String {
    run_gh(["api", "user", "--jq", ".login"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|username| !username.is_empty())
        .unwrap_or_default()
}
