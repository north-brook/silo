use crate::config::ConfigStore;
use std::process::Command;
use std::time::Instant;

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
    let command_line = args.join(" ");

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let output = Command::new("gh").args(&args).output().ok()?;
        log::debug!(
            "gh command completed success={} duration_ms={} args={command_line}",
            output.status.success(),
            started.elapsed().as_millis()
        );

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
    log::debug!("checking whether gh is installed");
    run_gh(["--version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_configured() -> bool {
    log::debug!("checking whether gh is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.gh.username.trim().is_empty() && !config.gh.token.trim().is_empty())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_username() -> String {
    log::debug!("reading gh username");
    run_gh(["api", "user", "--jq", ".login"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|username| !username.is_empty())
        .unwrap_or_default()
}
