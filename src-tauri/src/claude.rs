use crate::config::ConfigStore;
use std::process::Command;
use toml::Value;

const CLAUDE_OAUTH_TOKEN_PREFIX: &str = "sk-ant-oat01-";
const MIN_CLAUDE_OAUTH_TOKEN_LEN: usize = 100;

#[tauri::command]
pub async fn claude_authenticate() -> Result<(), String> {
    log::info!("starting claude authentication");
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
                extract_token(&combined).ok_or_else(|| {
                    "claude setup-token did not emit a valid OAuth token; output may have been truncated"
                        .to_string()
                })
            })
    })
    .await
    .map_err(|error| format!("claude setup-token task failed: {error}"))?
    .and_then(|token| {
        log::debug!("persisting claude token");
        ConfigStore::new()
            .map_err(|error| error.to_string())?
            .write("claude.token", Value::String(token))
            .map_err(|error| error.to_string())
    })?;
    log::info!("claude authentication saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn claude_configured() -> bool {
    log::trace!("checking whether claude is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.claude.token.trim().is_empty())
        .unwrap_or(false)
}

fn extract_token(output: &str) -> Option<String> {
    let lines = output.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        if let Some(token_start) = line.find(CLAUDE_OAUTH_TOKEN_PREFIX) {
            let mut candidate = trim_token_punctuation(&line[token_start..]).to_owned();

            for continuation in lines.iter().skip(index + 1) {
                let continuation = continuation.trim();
                if continuation.is_empty() || !continuation.chars().all(is_token_char) {
                    break;
                }
                candidate.push_str(continuation);
            }

            if looks_like_claude_token(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn looks_like_claude_token(segment: &str) -> bool {
    let token = trim_token_punctuation(segment);
    token.starts_with(CLAUDE_OAUTH_TOKEN_PREFIX) && token.len() >= MIN_CLAUDE_OAUTH_TOKEN_LEN
}

fn trim_token_punctuation(segment: &str) -> &str {
    let token = segment.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '-' && character != '_'
    });
    token
}

fn is_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '-' || character == '_'
}

#[cfg(test)]
mod tests {
    use super::extract_token;

    #[test]
    fn extracts_token_from_process_output() {
        let token = extract_token(
            "Your OAuth token (valid for 1 year):\n\nsk-ant-oat01-abc123_def4567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-abcdefghij1234567890abcdefghij\n",
        )
            .expect("token should be found");

        assert_eq!(
            token,
            "sk-ant-oat01-abc123_def4567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-abcdefghij1234567890abcdefghij"
        );
    }

    #[test]
    fn extracts_wrapped_token_from_process_output() {
        let token = extract_token(
            "✓ Long-lived authentication token created successfully!\n\nYour OAuth token (valid for 1 year):\n\nsk-ant-oat01-c63L10E5MpuZHhsh4HKRkT2CFR4SSxCqm4lYOaWEwTvhUL95PC8VCM1ee3p8UW9lkTK2hXjmx-f1M7Q7Q\nD3j4A-Iz8nLgAA\n\nStore this token securely.\n",
        )
        .expect("wrapped token should be found");

        assert_eq!(
            token,
            "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"
        );
    }

    #[test]
    fn rejects_truncated_token() {
        let token = extract_token(
            "Your OAuth token (valid for 1 year):\n\nsk-ant-oat01-KO_g5zR_Ay9uOpARQVIbUhzsNS_0jddaZAtVgG_l-wegj4vApjVF2sixWeQdFC4L7ZA\n",
        );

        assert!(token.is_none());
    }

    #[test]
    fn returns_none_when_output_has_no_token() {
        assert!(extract_token("setup complete").is_none());
    }
}
