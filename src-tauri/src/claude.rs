use crate::config::{set_claude_token, ConfigStore};
use std::process::Command;

#[tauri::command]
pub async fn claude_authenticate() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("claude")
            .arg("setup-token")
            .output()
            .map_err(|error| format!("failed to start claude setup-token: {error}"))
            .and_then(|output| {
                if !output.status.success() {
                    return Err(format!(
                        "claude setup-token exited with status {}",
                        output.status
                    ));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                extract_token(&combined)
                    .ok_or_else(|| "claude setup-token did not emit a token".to_string())
            })
    })
    .await
    .map_err(|error| format!("claude setup-token task failed: {error}"))?
    .and_then(|token| set_claude_token(token).map_err(|error| error.to_string()))
}

#[tauri::command]
pub async fn claude_configured() -> bool {
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.claude.token.trim().is_empty())
        .unwrap_or(false)
}

fn extract_token(output: &str) -> Option<String> {
    output.split_whitespace().find_map(clean_claude_token)
}

fn looks_like_claude_token(segment: &str) -> bool {
    let token = trim_token_punctuation(segment);
    token.starts_with("sk-ant-") && token.len() > "sk-ant-".len()
}

fn clean_claude_token(segment: &str) -> Option<String> {
    let token = trim_token_punctuation(segment);
    looks_like_claude_token(token).then(|| token.to_owned())
}

fn trim_token_punctuation(segment: &str) -> &str {
    let token = segment.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '-' && character != '_'
    });
    token
}

#[cfg(test)]
mod tests {
    use super::extract_token;

    #[test]
    fn extracts_token_from_process_output() {
        let token = extract_token("Your Claude token is sk-ant-abc123_def and is ready to use.")
            .expect("token should be found");

        assert_eq!(token, "sk-ant-abc123_def");
    }

    #[test]
    fn returns_none_when_output_has_no_token() {
        assert!(extract_token("setup complete").is_none());
    }
}
