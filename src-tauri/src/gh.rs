use crate::config::{ConfigStore, ProjectConfig};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn run_gh<I, S>(args: I) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    run_gh_in_dir(args, None::<&Path>).await
}

async fn run_gh_in_dir<I, S, P>(args: I, dir: Option<P>) -> Option<CommandResult>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
    P: AsRef<Path>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    let command_line = args.join(" ");
    let dir = dir.map(|path| path.as_ref().to_path_buf());

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let mut command = Command::new("gh");
        if let Some(dir) = dir {
            command.current_dir(dir);
        }
        let output = command.args(&args).output().ok()?;
        if output.status.success() {
            log::trace!(
                "gh command completed duration_ms={} args={command_line}",
                started.elapsed().as_millis()
            );
        } else {
            log::warn!(
                "gh command failed duration_ms={} args={} stderr={}",
                started.elapsed().as_millis(),
                command_line,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }

        Some(CommandResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub async fn gh_installed() -> bool {
    log::trace!("checking whether gh is installed");
    run_gh(["--version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_configured() -> bool {
    log::trace!("checking whether gh is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| !config.gh.username.trim().is_empty() && !config.gh.token.trim().is_empty())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn gh_username() -> String {
    log::trace!("reading gh username");
    run_gh(["api", "user", "--jq", ".login"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|username| !username.is_empty())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn gh_project_branches(project: String) -> Result<Vec<String>, String> {
    log::info!("listing GitHub branches for project {project}");
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project_config = config
        .projects
        .get(&project)
        .ok_or_else(|| format!("project not found: {project}"))?;

    let repo = gh_repo_name(project_config).await?;
    let result = run_gh([
        "api".to_string(),
        format!("repos/{repo}/branches"),
        "--paginate".to_string(),
        "--jq".to_string(),
        ".[].name".to_string(),
    ])
    .await
    .ok_or_else(|| "failed to execute gh".to_string())?;

    if !result.success {
        return Err(gh_error("failed to list project branches", &result.stderr));
    }

    let mut branches = parse_output_lines(&result.stdout);
    branches.sort();
    branches.dedup();
    log::info!(
        "listed {} GitHub branches for project {}",
        branches.len(),
        project
    );
    Ok(branches)
}

async fn gh_repo_name(project: &ProjectConfig) -> Result<String, String> {
    let result = run_gh_in_dir(
        [
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
        Some(Path::new(&project.path)),
    )
    .await
    .ok_or_else(|| "failed to execute gh".to_string())?;

    if !result.success {
        return Err(gh_error(
            "failed to resolve project GitHub repository",
            &result.stderr,
        ));
    }

    let repo = result.stdout.trim();
    if repo.is_empty() {
        return Err("failed to resolve project GitHub repository".to_string());
    }

    Ok(repo.to_string())
}

fn parse_output_lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn gh_error(context: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClaudeConfig, CodexConfig, GcloudConfig, GhConfig, SiloConfig};
    use indexmap::IndexMap;

    #[test]
    fn parse_output_lines_ignores_empty_entries() {
        assert_eq!(
            parse_output_lines("main\n\nfeature/test\n  \nrelease\n"),
            vec![
                "main".to_string(),
                "feature/test".to_string(),
                "release".to_string()
            ]
        );
    }

    #[test]
    fn project_lookup_uses_configured_name() {
        let config = SiloConfig {
            gcloud: GcloudConfig::default(),
            gh: GhConfig::default(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            projects: IndexMap::from_iter([(
                "demo".to_string(),
                ProjectConfig {
                    name: "demo".to_string(),
                    path: "/tmp/demo".to_string(),
                    image: None,
                    target_branch: "main".to_string(),
                    gcloud: Default::default(),
                },
            )]),
        };

        let project = config.projects.get("demo").expect("project should exist");
        assert_eq!(project.target_branch, "main");
        assert_eq!(project.path, "/tmp/demo");
    }
}
