use crate::config::{ConfigError, ConfigStore};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) fn detect_codex_auth_json(home_dir: &Path) -> Option<String> {
    let contents = codex_auth_file_contents(home_dir)?;
    normalize_codex_auth_json(&contents)
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

    let home_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| ConfigError::HomeDirectoryNotFound.to_string())?;
    let auth_json = detect_codex_auth_json(&home_dir)
        .ok_or_else(|| "codex login completed but auth.json could not be read".to_string())?;
    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    let mut config = store.load().map_err(|error| error.to_string())?;
    config.codex.auth_json = auth_json;
    store.save(&config).map_err(|error| error.to_string())?;
    log::info!("codex authentication saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn codex_configured() -> bool {
    log::trace!("checking whether codex is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| has_configured_auth_json(&config.codex.auth_json))
        .unwrap_or(false)
}

pub(crate) fn codex_token_from_auth_json(contents: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(contents).ok()?;
    codex_auth_credential(&json)
}

pub(crate) fn normalize_codex_auth_json(contents: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(contents).ok()?;
    codex_auth_credential(&json)?;
    Some(json.to_string())
}

fn has_configured_auth_json(contents: &str) -> bool {
    normalize_codex_auth_json(contents).is_some()
}

fn codex_auth_file_contents(home_dir: &Path) -> Option<String> {
    let auth_path = home_dir.join(".codex").join("auth.json");
    fs::read_to_string(auth_path).ok()
}

fn codex_auth_credential(json: &serde_json::Value) -> Option<String> {
    json.get("OPENAI_API_KEY")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_value)
        .or_else(|| {
            json.get("tokens")
                .and_then(|tokens| tokens.get("refresh_token"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_value)
        })
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
    use super::{codex_token_from_auth_json, detect_codex_auth_json, has_configured_auth_json};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn codex_token_prefers_refresh_token_when_present() {
        let token = codex_token_from_auth_json(
            "{\"tokens\":{\"access_token\":\"codex-access-token\",\"refresh_token\":\"codex-refresh-token\"}}",
        )
        .expect("token should be detected");

        assert_eq!(token, "codex-refresh-token");
    }

    #[test]
    fn codex_token_falls_back_to_access_token_when_refresh_token_is_missing() {
        let token =
            codex_token_from_auth_json("{\"tokens\":{\"access_token\":\"codex-access-token\"}}")
                .expect("token should be detected");

        assert_eq!(token, "codex-access-token");
    }

    #[test]
    fn codex_auth_json_can_be_read_from_auth_json() {
        let temp_dir = TestDir::new();
        let codex_dir = temp_dir.root.join(".codex");
        fs::create_dir_all(&codex_dir).expect("codex dir should be created");
        fs::write(
            codex_dir.join("auth.json"),
            "{\n  \"tokens\": {\n    \"access_token\": \"codex-access-token\",\n    \"refresh_token\": \"codex-refresh-token\"\n  }\n}",
        )
        .expect("auth file should be written");

        let auth_json = detect_codex_auth_json(&temp_dir.root).expect("auth json should be read");
        assert_eq!(
            auth_json,
            "{\"tokens\":{\"access_token\":\"codex-access-token\",\"refresh_token\":\"codex-refresh-token\"}}"
        );
    }

    #[test]
    fn configured_auth_json_requires_a_valid_credential_payload() {
        assert!(has_configured_auth_json(
            "{\"tokens\":{\"refresh_token\":\"codex-refresh-token\"}}"
        ));
        assert!(!has_configured_auth_json(""));
        assert!(!has_configured_auth_json("{\"tokens\":{}}"));
        assert!(!has_configured_auth_json("not json"));
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
