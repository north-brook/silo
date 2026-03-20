use std::process::Command;

use crate::files::workspace_root;

#[derive(Debug, Clone, Default)]
pub(crate) struct ZmxSession {
    pub(crate) name: String,
    pub(crate) command: Option<String>,
}

pub(crate) fn list_zmx_sessions() -> Result<Vec<ZmxSession>, String> {
    let output = Command::new("bash")
        .args(["-lc", "zmx list"])
        .output()
        .map_err(|error| format!("spawn failed: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "exit status {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }

    parse_zmx_sessions(&String::from_utf8_lossy(&output.stdout))
}

pub(crate) fn parse_zmx_session(line: &str) -> Option<ZmxSession> {
    let mut name = None;
    let mut command = None;
    for field in line.split('\t') {
        let (key, value) = field.split_once('=')?;
        match key {
            "name" | "session_name" => name = Some(value.to_string()),
            "cmd" => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    command = Some(trimmed.to_string());
                }
            }
            _ => {}
        }
    }

    Some(ZmxSession {
        name: name?,
        command,
    })
}

pub(crate) fn parse_zmx_sessions(stdout: &str) -> Result<Vec<ZmxSession>, String> {
    let mut sessions = Vec::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("no sessions found ") {
            continue;
        }

        let session = parse_zmx_session(line)
            .ok_or_else(|| format!("failed to parse zmx session line: {line}"))?;
        sessions.push(session);
    }

    Ok(sessions)
}

pub(crate) fn read_workspace_branch() -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root())
        .args(["branch", "--show-current"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty()).then_some(branch)
}
