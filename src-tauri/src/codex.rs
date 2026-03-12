use crate::config::{ConfigError, ConfigStore};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use toml::Value;

pub(crate) fn detect_codex_token(home_dir: &Path) -> Option<String> {
    codex_auth_token(home_dir)
}

pub(crate) fn detect_codex_token_from_store() -> Result<String, ConfigError> {
    let home_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(ConfigError::HomeDirectoryNotFound)?;

    Ok(detect_codex_token(&home_dir).unwrap_or_default())
}

#[tauri::command]
pub async fn codex_authenticate() -> Result<(), String> {
    log::info!("starting codex authentication");
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
    ConfigStore::new()
        .map_err(|error| error.to_string())?
        .write("codex.token", Value::String(token))
        .map_err(|error| error.to_string())?;
    log::info!("codex authentication saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn codex_configured() -> bool {
    log::trace!("checking whether codex is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.codex.token.trim().is_empty())
        .unwrap_or(false)
}

fn codex_auth_token(home_dir: &Path) -> Option<String> {
    let auth_path = home_dir.join(".codex").join("auth.json");
    let contents = fs::read_to_string(auth_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&contents).ok()?;

    json.get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_value)
        .or_else(|| {
            json.get("tokens")
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_value)
        })
}

fn normalize_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "(unset)" || trimmed == "unset" {
        return None;
    }

    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::codex_auth_token;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn codex_token_can_be_read_from_auth_json() {
        let temp_dir = TestDir::new();
        let codex_dir = temp_dir.root.join(".codex");
        fs::create_dir_all(&codex_dir).expect("codex dir should be created");
        fs::write(
            codex_dir.join("auth.json"),
            "{\"tokens\":{\"access_token\":\"codex-access-token\"}}",
        )
        .expect("auth file should be written");

        let token = codex_auth_token(&temp_dir.root).expect("token should be detected");
        assert_eq!(token, "codex-access-token");
    }

    struct TestDir {
        root: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "silo-codex-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let root = env::temp_dir().join(unique);
            fs::create_dir_all(&root).expect("test dir should be created");

            Self { root }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
