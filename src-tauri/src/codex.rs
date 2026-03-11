use crate::config::{detect_codex_token_from_store, set_codex_token, ConfigStore};
use std::process::{Command, Stdio};

#[tauri::command]
pub async fn codex_authenticate() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("codex")
            .arg("login")
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| format!("failed to start codex login: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("codex login exited with status {status}"))
                }
            })
    })
    .await
    .map_err(|error| format!("codex login task failed: {error}"))??;

    let token = detect_codex_token_from_store().map_err(|error| error.to_string())?;
    set_codex_token(token).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn codex_configured() -> bool {
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.codex.token.trim().is_empty())
        .unwrap_or(false)
}
