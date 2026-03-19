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

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_zmx_session)
        .collect())
}

pub(crate) fn parse_zmx_session(line: &str) -> Option<ZmxSession> {
    let mut name = None;
    let mut command = None;
    for field in line.split('\t') {
        let (key, value) = field.split_once('=')?;
        match key {
            "session_name" => name = Some(value.to_string()),
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
