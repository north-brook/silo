use crate::config::{ConfigStore, ProjectConfig};
use crate::prompts;
use crate::terminal;
use crate::terminal::TerminalManager;
use crate::workspaces::{self, WorkspaceLookup};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;
use tauri::State;
use toml::Value as TomlValue;

const MAX_CHECK_LOG_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
struct CommandResult {
    success: bool,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone)]
struct BranchWorkspaceContext {
    lookup: WorkspaceLookup,
    target_branch: String,
}

#[derive(Debug, Clone)]
struct PullRequestSummary {
    number: u64,
    head_ref_oid: String,
    status: String,
    updated_at: Option<String>,
    url: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Diff {
    overview: DiffOverview,
    local: DiffSection,
    remote: DiffSection,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiffSection {
    overview: DiffOverview,
    files: Vec<DiffFile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiffOverview {
    additions: u64,
    deletions: u64,
    files_changed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiffFile {
    path: String,
    previous_path: Option<String>,
    status: String,
    additions: u64,
    deletions: u64,
    binary: bool,
    patch: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Deployment {
    id: String,
    environment: String,
    state: String,
    description: String,
    url: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Check {
    id: String,
    name: String,
    workflow: Option<String>,
    state: CheckState,
    bucket: Option<String>,
    description: Option<String>,
    link: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    log_excerpt: String,
    log_truncated: bool,
    log_available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Queued,
    InProgress,
    Pending,
    Requested,
    Waiting,
    Success,
    Failure,
    Cancelled,
    Skipped,
    Neutral,
    ActionRequired,
    TimedOut,
    StartupFailure,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PullRequestObservation {
    title: Option<String>,
    body: Option<String>,
    deployments: Vec<Deployment>,
    checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PullRequestStatus {
    status: String,
    number: u64,
    url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitTerminalResult {
    attachment_id: String,
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
pub async fn git_authenticate() -> Result<(), String> {
    log::info!("starting git authentication");
    tauri::async_runtime::spawn_blocking(move || {
        Command::new("gh")
            .args(["auth", "login"])
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| format!("failed to start gh auth login: {error}"))
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("gh auth login exited with status {status}"))
                }
            })
    })
    .await
    .map_err(|error| format!("gh auth login task failed: {error}"))??;

    let username = git_username().await;
    if username.trim().is_empty() {
        return Err("gh auth login completed but no GitHub username was detected".to_string());
    }

    let token = run_gh(["auth", "token"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "gh auth login completed but no GitHub token was detected".to_string())?;

    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    store
        .write("git.gh_username", TomlValue::String(username))
        .map_err(|error| error.to_string())?;
    store
        .write("git.gh_token", TomlValue::String(token))
        .map_err(|error| error.to_string())?;
    log::info!("git authentication saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn git_installed() -> bool {
    log::trace!("checking whether gh is installed");
    run_gh(["--version"])
        .await
        .map(|result| result.success)
        .unwrap_or(false)
}

#[tauri::command]
pub async fn git_configured() -> bool {
    log::trace!("checking whether git is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| {
            !config.git.gh_username.trim().is_empty() && !config.git.gh_token.trim().is_empty()
        })
        .unwrap_or(false)
}

#[tauri::command]
pub async fn git_username() -> String {
    log::trace!("reading gh username");
    run_gh(["api", "user", "--jq", ".login"])
        .await
        .filter(|result| result.success)
        .map(|result| result.stdout.trim().to_owned())
        .filter(|username| !username.is_empty())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn git_project_branches(project: String) -> Result<Vec<String>, String> {
    log::info!("listing GitHub branches for project {project}");
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project_config = config
        .projects
        .get(&project)
        .ok_or_else(|| format!("project not found: {project}"))?;

    let repo = git_repo_name(project_config).await?;
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
        return Err(git_error("failed to list project branches", &result.stderr));
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

#[tauri::command]
pub async fn git_diff(workspace: String) -> Result<Diff, String> {
    let context = branch_workspace_context(&workspace).await?;
    let result =
        run_workspace_command(&context, &diff_remote_command(&context.target_branch)).await?;
    if !result.success {
        return Err(git_error(
            "failed to collect workspace diff",
            &result.stderr,
        ));
    }

    parse_diff(&result.stdout)
}

#[tauri::command]
pub async fn git_update_branch(workspace: String, branch: String) -> Result<(), String> {
    log::info!("updating workspace branch for {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let branch = trim_branch_input(&branch, "branch")?;
    ensure_workspace_tree_clean(&context, "rename workspace branch").await?;

    let current_branch = current_workspace_branch(&context).await?;
    let metadata_branch = context
        .lookup
        .workspace
        .branch_name()
        .unwrap_or_default()
        .trim();
    if branch == current_branch {
        if branch == metadata_branch {
            return Ok(());
        }
        return workspaces::set_workspace_metadata(&workspace, "branch", &branch).await;
    }

    let result = run_workspace_command(&context, &rename_branch_remote_command(&branch)).await?;
    if !result.success {
        return Err(git_error(
            "failed to rename workspace branch",
            &result.stderr,
        ));
    }

    workspaces::set_workspace_metadata(&workspace, "branch", &branch).await
}

#[tauri::command]
pub async fn git_update_target_branch(
    workspace: String,
    target_branch: String,
) -> Result<(), String> {
    log::info!("updating workspace target branch for {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let target_branch = trim_branch_input(&target_branch, "target branch")?;
    ensure_workspace_tree_clean(&context, "retarget workspace branch").await?;

    if target_branch == context.target_branch {
        return Ok(());
    }

    let result =
        run_workspace_command(&context, &retarget_branch_remote_command(&target_branch)).await?;
    if !result.success {
        return Err(git_error(
            "failed to rebase workspace branch onto target branch",
            &result.stderr,
        ));
    }

    workspaces::set_workspace_metadata(&workspace, "target_branch", &target_branch).await
}

#[tauri::command]
pub async fn git_pr_status(workspace: String) -> Result<Option<PullRequestStatus>, String> {
    log::info!("checking PR status for workspace {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    Ok(find_pull_request(&context)
        .await?
        .map(|pr| PullRequestStatus {
            status: pr.status,
            number: pr.number,
            url: pr.url,
        }))
}

#[tauri::command]
pub async fn git_tree_dirty(workspace: String) -> Result<bool, String> {
    log::info!("checking tree dirtiness for workspace {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    workspace_tree_dirty(&context).await
}

#[tauri::command]
pub async fn git_pr_observe(workspace: String) -> Result<Option<PullRequestObservation>, String> {
    let context = branch_workspace_context(&workspace).await?;
    let Some(pr) = find_open_pull_request(&context).await? else {
        return Ok(None);
    };

    let deployments = fetch_pr_deployments(&context, &pr).await?;
    let checks = fetch_pr_checks(&context, &pr).await?;
    Ok(Some(PullRequestObservation {
        title: Some(pr.title),
        body: Some(pr.body),
        deployments,
        checks,
    }))
}

#[tauri::command]
pub async fn git_push(
    state: State<'_, TerminalManager>,
    workspace: String,
) -> Result<GitTerminalResult, String> {
    log::info!("pushing workspace branch for {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let branch = current_workspace_branch(&context).await?;
    let prompt = prompts::git_push_prompt(&branch, &context.target_branch);
    let command = terminal::codex_prompt_command(&prompt);
    let attachment_id =
        terminal::start_terminal_command(state.inner(), &workspace, &command).await?;

    Ok(GitTerminalResult { attachment_id })
}

#[tauri::command]
pub async fn git_merge_pr(workspace: String) -> Result<(), String> {
    log::info!("merging PR for workspace {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let branch = current_workspace_branch(&context).await?;
    let pr = find_open_pull_request(&context)
        .await?
        .ok_or_else(|| format!("no open pull request found for branch {}", branch))?;

    let command = format!(
        "gh pr merge {} --merge --match-head-commit {}",
        pr.number,
        shell_quote(&pr.head_ref_oid)
    );
    let result = run_workspace_command(&context, &command).await?;
    if !result.success {
        return Err(git_error("failed to merge pull request", &result.stderr));
    }

    Ok(())
}

#[tauri::command]
pub async fn git_rerun_failed_checks(workspace: String) -> Result<(), String> {
    log::info!("rerunning failed checks for workspace {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let branch = current_workspace_branch(&context).await?;
    let pr = find_open_pull_request(&context)
        .await?
        .ok_or_else(|| format!("no open pull request found for branch {}", branch))?;

    let mut run_ids: Vec<String> = fetch_pr_checks(&context, &pr)
        .await?
        .into_iter()
        .filter(|check| is_rerunnable_failed_check_state(check.state))
        .filter_map(|check| check.link.as_deref().and_then(github_actions_run_id))
        .collect();
    run_ids.sort_unstable();
    run_ids.dedup();

    if run_ids.is_empty() {
        return Err("no failed GitHub Actions checks to rerun".to_string());
    }

    for run_id in run_ids {
        let result =
            run_workspace_command(&context, &rerun_failed_checks_remote_command(&run_id)).await?;
        if !result.success {
            return Err(git_error(
                "failed to rerun failed pull request checks",
                &result.stderr,
            ));
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn git_create_pr(
    state: State<'_, TerminalManager>,
    workspace: String,
) -> Result<GitTerminalResult, String> {
    log::info!("creating pull request for workspace {workspace}");
    let context = branch_workspace_context(&workspace).await?;
    let branch = current_workspace_branch(&context).await?;
    let prompt = prompts::git_create_pr_prompt(&branch, &context.target_branch);
    let command = terminal::codex_prompt_command(&prompt);
    let attachment_id =
        terminal::start_terminal_command(state.inner(), &workspace, &command).await?;

    Ok(GitTerminalResult { attachment_id })
}

async fn git_repo_name(project: &ProjectConfig) -> Result<String, String> {
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
        return Err(git_error(
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

async fn branch_workspace_context(workspace: &str) -> Result<BranchWorkspaceContext, String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if lookup.workspace.is_template() {
        return Err(format!(
            "workspace {} is a template workspace and does not support git operations",
            workspace
        ));
    }

    let target_branch = lookup
        .workspace
        .target_branch()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("workspace {} is missing target branch metadata", workspace))?
        .to_string();

    Ok(BranchWorkspaceContext {
        lookup,
        target_branch,
    })
}

async fn run_workspace_command(
    context: &BranchWorkspaceContext,
    command: &str,
) -> Result<terminal::CommandResult, String> {
    let remote_command = terminal::workspace_shell_command_with_credentials(command);
    terminal::run_remote_command(&context.lookup, &remote_command).await
}

async fn workspace_tree_dirty(context: &BranchWorkspaceContext) -> Result<bool, String> {
    let result = run_workspace_command(context, tree_dirty_remote_command()).await?;
    if !result.success {
        return Err(git_error(
            "failed to check workspace tree dirtiness",
            &result.stderr,
        ));
    }

    parse_tree_dirty_output(&result.stdout)
}

async fn ensure_workspace_tree_clean(
    context: &BranchWorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    if workspace_tree_dirty(context).await? {
        return Err(format!(
            "cannot {operation}: workspace has uncommitted changes"
        ));
    }

    Ok(())
}

async fn current_workspace_branch(context: &BranchWorkspaceContext) -> Result<String, String> {
    let result = run_workspace_command(context, "git branch --show-current").await?;
    if !result.success {
        return Err(git_error(
            "failed to resolve workspace branch",
            &result.stderr,
        ));
    }

    let branch = result.stdout.trim();
    if branch.is_empty() {
        return Err("failed to resolve workspace branch".to_string());
    }

    Ok(branch.to_string())
}

async fn find_open_pull_request(
    context: &BranchWorkspaceContext,
) -> Result<Option<PullRequestSummary>, String> {
    Ok(find_pull_request(context)
        .await?
        .filter(|pull_request| pull_request.status == "open"))
}

async fn find_pull_request(
    context: &BranchWorkspaceContext,
) -> Result<Option<PullRequestSummary>, String> {
    let branch = current_workspace_branch(context).await?;

    if let Some(pull_request) = find_current_pull_request(context, &branch).await? {
        return Ok(Some(pull_request));
    }

    find_historical_pull_request(context, &branch).await
}

async fn find_current_pull_request(
    context: &BranchWorkspaceContext,
    branch: &str,
) -> Result<Option<PullRequestSummary>, String> {
    let command = "gh pr view --json number,headRefName,baseRefName,headRefOid,state,mergedAt,updatedAt,url,title,body";
    let result = run_workspace_command(context, command).await?;
    if !result.success {
        if is_missing_pull_request_error(&result.stderr) {
            return Ok(None);
        }
        return Err(git_error("failed to inspect pull request", &result.stderr));
    }

    parse_pull_request_view(&result.stdout, branch, &context.target_branch)
}

async fn find_historical_pull_request(
    context: &BranchWorkspaceContext,
    branch: &str,
) -> Result<Option<PullRequestSummary>, String> {
    let command = format!(
        "BRANCH={branch}\n\
TARGET_BRANCH={target_branch}\n\
REPO_OWNER=\"$(gh repo view --json owner --jq '.owner.login')\"\n\
gh api --method GET repos/{{owner}}/{{repo}}/pulls -f state=closed -f head=\"$REPO_OWNER:$BRANCH\" -f base=\"$TARGET_BRANCH\" -f per_page=100 --jq 'map({{number, headRefName: .head.ref, baseRefName: .base.ref, headRefOid: .head.sha, state: .state, mergedAt: .merged_at, updatedAt: .updated_at, url: .html_url, title, body}})'",
        branch = shell_quote(branch),
        target_branch = shell_quote(&context.target_branch),
    );
    let result = run_workspace_command(context, &command).await?;
    if !result.success {
        return Err(git_error("failed to list pull requests", &result.stderr));
    }

    parse_pull_requests(&result.stdout, branch, &context.target_branch)
}

async fn fetch_pr_checks(
    context: &BranchWorkspaceContext,
    pr: &PullRequestSummary,
) -> Result<Vec<Check>, String> {
    let command = format!(
        "gh pr checks {} --json bucket,completedAt,description,event,link,name,startedAt,state,workflow || status=$?; if [ \"${{status:-0}}\" -ne 0 ] && [ \"${{status:-0}}\" -ne 8 ]; then exit \"${{status}}\"; fi",
        pr.number
    );
    let result = run_workspace_command(context, &command).await?;
    if !result.success {
        return Err(git_error(
            "failed to list pull request checks",
            &result.stderr,
        ));
    }

    let checks = parse_checks_json(&result.stdout)?;
    let mut observed = Vec::with_capacity(checks.len());
    for check in checks {
        observed.push(enrich_check_with_logs(context, check).await);
    }
    Ok(observed)
}

async fn enrich_check_with_logs(context: &BranchWorkspaceContext, mut check: Check) -> Check {
    let Some(link) = check.link.as_deref() else {
        return check;
    };
    let Some(job_id) = github_actions_job_id(link) else {
        return check;
    };

    let command = format!("gh run view --log --job {}", shell_quote(&job_id));
    match run_workspace_command(context, &command).await {
        Ok(result) if result.success => {
            let (excerpt, truncated) = truncate_log_excerpt(&result.stdout);
            check.log_excerpt = excerpt;
            check.log_truncated = truncated;
            check.log_available = true;
        }
        Ok(result) => {
            log::warn!(
                "failed to collect check logs for job {}: {}",
                job_id,
                result.stderr.trim()
            );
        }
        Err(error) => {
            log::warn!("failed to collect check logs for job {}: {}", job_id, error);
        }
    }

    check
}

async fn fetch_pr_deployments(
    context: &BranchWorkspaceContext,
    pr: &PullRequestSummary,
) -> Result<Vec<Deployment>, String> {
    let command = format!(
        "gh api --method GET repos/{{owner}}/{{repo}}/deployments -f sha={} -f per_page=100",
        shell_quote(&pr.head_ref_oid)
    );
    let result = run_workspace_command(context, &command).await?;
    if !result.success {
        return Err(git_error(
            "failed to list pull request deployments",
            &result.stderr,
        ));
    }

    let mut deployments = parse_deployments_json(&result.stdout)?;
    for deployment in &mut deployments {
        fill_deployment_status(context, deployment).await?;
    }
    deployments.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.environment.cmp(&right.environment))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(deployments)
}

async fn fill_deployment_status(
    context: &BranchWorkspaceContext,
    deployment: &mut Deployment,
) -> Result<(), String> {
    let command = format!(
        "gh api --method GET repos/{{owner}}/{{repo}}/deployments/{}/statuses -f per_page=100",
        deployment.id
    );
    let result = run_workspace_command(context, &command).await?;
    if !result.success {
        return Err(git_error(
            "failed to list deployment statuses",
            &result.stderr,
        ));
    }

    let Some(status) = latest_deployment_status(&result.stdout)? else {
        return Ok(());
    };

    if let Some(state) = string_field(&status, "state") {
        deployment.state = state;
    }
    if deployment.description.is_empty() {
        deployment.description = string_field(&status, "description").unwrap_or_default();
    }
    if deployment.url.is_none() {
        deployment.url = string_field(&status, "environment_url")
            .or_else(|| string_field(&status, "log_url"))
            .filter(|value| !value.is_empty());
    }
    if deployment.updated_at.is_none() {
        deployment.updated_at = string_field(&status, "updated_at");
    }

    Ok(())
}

fn latest_deployment_status(stdout: &str) -> Result<Option<Value>, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse deployment statuses: {error}"))?;
    let statuses = value
        .as_array()
        .ok_or_else(|| "deployment statuses response was not an array".to_string())?;
    Ok(statuses
        .iter()
        .max_by(|left, right| {
            let left_time = string_field(left, "updated_at")
                .or_else(|| string_field(left, "created_at"))
                .unwrap_or_default();
            let right_time = string_field(right, "updated_at")
                .or_else(|| string_field(right, "created_at"))
                .unwrap_or_default();
            left_time
                .cmp(&right_time)
                .then_with(|| json_u64_field(left, "id").cmp(&json_u64_field(right, "id")))
        })
        .cloned())
}

fn diff_remote_command(target_branch: &str) -> String {
    format!(
        "TARGET_BRANCH={target_branch}\n\
git fetch --quiet origin \"$TARGET_BRANCH\"\n\
MERGE_BASE=\"$(git merge-base HEAD \"origin/$TARGET_BRANCH\")\"\n\
printf \"===REMOTE_DIFF===\\n\"\n\
git -c core.quotepath=false diff --find-renames --binary --patch --no-color --no-ext-diff \"$MERGE_BASE\" HEAD\n\
printf \"===LOCAL_DIFF===\\n\"\n\
git -c core.quotepath=false diff --find-renames --binary --patch --no-color --no-ext-diff HEAD\n\
while IFS= read -r -d \"\" path; do\n\
  if ! git -c core.quotepath=false diff --no-index --binary --patch --no-color --no-ext-diff -- /dev/null \"$path\"; then\n\
    status=$?\n\
    if [ \"$status\" -gt 1 ]; then\n\
      exit \"$status\"\n\
    fi\n\
  fi\n\
done < <(git -c core.quotepath=false ls-files --others --exclude-standard -z)",
        target_branch = shell_quote(target_branch),
    )
}

fn rename_branch_remote_command(branch: &str) -> String {
    format!(
        "BRANCH={branch}\n\
git check-ref-format --branch \"$BRANCH\" >/dev/null\n\
git branch -m \"$BRANCH\"",
        branch = shell_quote(branch),
    )
}

fn retarget_branch_remote_command(target_branch: &str) -> String {
    format!(
        "TARGET_BRANCH={target_branch}\n\
git check-ref-format --branch \"$TARGET_BRANCH\" >/dev/null\n\
git fetch --quiet origin \"$TARGET_BRANCH\"\n\
git rebase \"origin/$TARGET_BRANCH\"",
        target_branch = shell_quote(target_branch),
    )
}

fn rerun_failed_checks_remote_command(run_id: &str) -> String {
    format!("gh run rerun {} --failed", shell_quote(run_id))
}

fn tree_dirty_remote_command() -> &'static str {
    "git update-index -q --refresh\n\
if ! git diff --quiet --ignore-submodules -- || ! git diff --cached --quiet --ignore-submodules -- || [ -n \"$(git ls-files --others --exclude-standard)\" ]; then\n\
  printf 'true\\n'\n\
else\n\
  printf 'false\\n'\n\
fi"
}

fn trim_branch_input(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty"));
    }

    Ok(trimmed.to_string())
}

fn parse_tree_dirty_output(stdout: &str) -> Result<bool, String> {
    match stdout.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        value => Err(format!(
            "failed to check workspace tree dirtiness: unexpected output {value:?}"
        )),
    }
}

fn parse_diff(stdout: &str) -> Result<Diff, String> {
    let (remote_stdout, local_stdout) = split_diff_output(stdout)?;
    let remote = parse_diff_section(&remote_stdout)?;
    let local = parse_diff_section(&local_stdout)?;

    let additions = remote.overview.additions + local.overview.additions;
    let deletions = remote.overview.deletions + local.overview.deletions;
    let files_changed = remote
        .files
        .iter()
        .chain(local.files.iter())
        .map(|file| file.path.clone())
        .collect::<HashSet<_>>()
        .len();

    Ok(Diff {
        overview: DiffOverview {
            additions,
            deletions,
            files_changed,
        },
        local,
        remote,
    })
}

fn parse_diff_section(stdout: &str) -> Result<DiffSection, String> {
    let mut files = Vec::new();
    let mut additions = 0_u64;
    let mut deletions = 0_u64;

    for section in split_diff_sections(stdout)? {
        let file = parse_diff_file(&section)?;
        additions += file.additions;
        deletions += file.deletions;
        files.push(file);
    }

    Ok(DiffSection {
        overview: DiffOverview {
            additions,
            deletions,
            files_changed: files.len(),
        },
        files,
    })
}

fn split_diff_output(stdout: &str) -> Result<(String, String), String> {
    let (_, after_remote_marker) = stdout
        .split_once("===REMOTE_DIFF===\n")
        .ok_or_else(|| "failed to parse workspace diff output".to_string())?;
    let (remote, local) = after_remote_marker
        .split_once("===LOCAL_DIFF===\n")
        .ok_or_else(|| "failed to parse workspace diff output".to_string())?;
    Ok((remote.to_string(), local.to_string()))
}

fn split_diff_sections(stdout: &str) -> Result<Vec<String>, String> {
    if stdout.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut sections = Vec::new();
    let mut current = String::new();
    for segment in stdout.split_inclusive('\n') {
        if segment.starts_with("diff --git ") && !current.is_empty() {
            sections.push(current);
            current = String::new();
        }
        current.push_str(segment);
    }
    if !current.is_empty() {
        sections.push(current);
    }

    if sections
        .iter()
        .any(|section| !section.starts_with("diff --git "))
    {
        return Err("failed to parse git diff output".to_string());
    }

    Ok(sections)
}

fn parse_diff_file(section: &str) -> Result<DiffFile, String> {
    let mut path: Option<String> = None;
    let mut previous_path: Option<String> = None;
    let mut status = "modified".to_string();
    let mut binary = false;
    let mut additions = 0_u64;
    let mut deletions = 0_u64;

    for line in section.lines() {
        if line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
            binary = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("rename from ") {
            previous_path = Some(parse_git_path_value(value)?);
            status = "renamed".to_string();
            continue;
        }
        if let Some(value) = line.strip_prefix("rename to ") {
            path = Some(parse_git_path_value(value)?);
            status = "renamed".to_string();
            continue;
        }
        if let Some(value) = line.strip_prefix("copy from ") {
            previous_path = Some(parse_git_path_value(value)?);
            status = "copied".to_string();
            continue;
        }
        if let Some(value) = line.strip_prefix("copy to ") {
            path = Some(parse_git_path_value(value)?);
            status = "copied".to_string();
            continue;
        }
        if line.starts_with("new file mode ") {
            status = "added".to_string();
            continue;
        }
        if line.starts_with("deleted file mode ") {
            status = "deleted".to_string();
            continue;
        }
        if line.starts_with("old mode ") || line.starts_with("new mode ") {
            if status == "modified" {
                status = "type_changed".to_string();
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("+++ ") {
            let parsed = parse_patch_path(value)?;
            if parsed != "/dev/null" {
                path = Some(strip_diff_prefix(&parsed));
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("--- ") {
            let parsed = parse_patch_path(value)?;
            if parsed != "/dev/null"
                && previous_path.is_none()
                && matches!(status.as_str(), "renamed" | "copied" | "deleted")
            {
                previous_path = Some(strip_diff_prefix(&parsed));
            }
            continue;
        }
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }

    if path.is_none() || previous_path.is_none() {
        let (left, right) = parse_diff_git_header(section.lines().next().unwrap_or_default())?;
        if previous_path.is_none()
            && matches!(status.as_str(), "renamed" | "copied" | "deleted")
            && left != "/dev/null"
        {
            previous_path = Some(strip_diff_prefix(&left));
        }
        if path.is_none() && right != "/dev/null" {
            path = Some(strip_diff_prefix(&right));
        }
    }

    let path = match status.as_str() {
        "deleted" => previous_path.clone().or(path),
        _ => path.or_else(|| previous_path.clone()),
    }
    .ok_or_else(|| "failed to determine diff file path".to_string())?;

    let patch = if section.trim().is_empty() {
        None
    } else {
        Some(section.to_string())
    };

    Ok(DiffFile {
        path,
        previous_path,
        status,
        additions,
        deletions,
        binary,
        patch,
    })
}

fn parse_pull_requests(
    stdout: &str,
    branch: &str,
    target_branch: &str,
) -> Result<Option<PullRequestSummary>, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse pull request response: {error}"))?;
    let pulls = value
        .as_array()
        .ok_or_else(|| "pull request response was not an array".to_string())?;

    let mut matches = Vec::new();
    let mut open_matches = 0_usize;
    for item in pulls {
        let head_ref_name = string_field(item, "headRefName").unwrap_or_default();
        let base_ref_name = string_field(item, "baseRefName").unwrap_or_default();
        if head_ref_name != branch || base_ref_name != target_branch {
            continue;
        }

        let status = if string_field(item, "mergedAt").is_some() {
            "merged".to_string()
        } else {
            string_field(item, "state")
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_else(|| "closed".to_string())
        };
        if status == "open" {
            open_matches += 1;
        }
        matches.push(PullRequestSummary {
            number: json_u64_field(item, "number")
                .ok_or_else(|| "pull request was missing number".to_string())?,
            head_ref_oid: string_field(item, "headRefOid")
                .ok_or_else(|| "pull request was missing headRefOid".to_string())?,
            status,
            updated_at: string_field(item, "updatedAt"),
            url: string_field(item, "url")
                .ok_or_else(|| "pull request was missing url".to_string())?,
            title: string_field(item, "title")
                .ok_or_else(|| "pull request was missing title".to_string())?,
            body: item
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }

    if open_matches > 1 {
        return Err(format!(
            "multiple open pull requests matched branch {} against {}",
            branch, target_branch
        ));
    }

    matches.sort_by(|left, right| {
        let left_rank = pull_request_status_rank(&left.status);
        let right_rank = pull_request_status_rank(&right.status);
        right_rank
            .cmp(&left_rank)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.number.cmp(&left.number))
    });

    Ok(matches.into_iter().next())
}

fn parse_pull_request_view(
    stdout: &str,
    branch: &str,
    target_branch: &str,
) -> Result<Option<PullRequestSummary>, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse pull request response: {error}"))?;
    let head_ref_name = string_field(&value, "headRefName").unwrap_or_default();
    let base_ref_name = string_field(&value, "baseRefName").unwrap_or_default();
    if head_ref_name != branch || base_ref_name != target_branch {
        return Ok(None);
    }

    let status = if string_field(&value, "mergedAt").is_some() {
        "merged".to_string()
    } else {
        string_field(&value, "state")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_else(|| "closed".to_string())
    };

    Ok(Some(PullRequestSummary {
        number: json_u64_field(&value, "number")
            .ok_or_else(|| "pull request was missing number".to_string())?,
        head_ref_oid: string_field(&value, "headRefOid")
            .ok_or_else(|| "pull request was missing headRefOid".to_string())?,
        status,
        updated_at: string_field(&value, "updatedAt"),
        url: string_field(&value, "url")
            .ok_or_else(|| "pull request was missing url".to_string())?,
        title: string_field(&value, "title")
            .ok_or_else(|| "pull request was missing title".to_string())?,
        body: value
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }))
}

fn pull_request_status_rank(status: &str) -> u8 {
    match status {
        "open" => 3,
        "merged" => 2,
        "closed" => 1,
        _ => 0,
    }
}

fn parse_checks_json(stdout: &str) -> Result<Vec<Check>, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse check response: {error}"))?;
    let checks = value
        .as_array()
        .ok_or_else(|| "check response was not an array".to_string())?;
    Ok(checks
        .iter()
        .map(|check| Check {
            id: string_field(check, "link")
                .or_else(|| string_field(check, "name"))
                .unwrap_or_else(|| "check".to_string()),
            name: string_field(check, "name").unwrap_or_else(|| "Check".to_string()),
            workflow: string_field(check, "workflow"),
            state: parse_check_state(string_field(check, "state").as_deref()),
            bucket: string_field(check, "bucket"),
            description: string_field(check, "description").filter(|value| !value.is_empty()),
            link: string_field(check, "link").filter(|value| !value.is_empty()),
            started_at: string_field(check, "startedAt"),
            completed_at: string_field(check, "completedAt"),
            log_excerpt: String::new(),
            log_truncated: false,
            log_available: false,
        })
        .collect())
}

fn parse_deployments_json(stdout: &str) -> Result<Vec<Deployment>, String> {
    let value: Value = serde_json::from_str(stdout)
        .map_err(|error| format!("failed to parse deployments response: {error}"))?;
    let deployments = value
        .as_array()
        .ok_or_else(|| "deployments response was not an array".to_string())?;

    Ok(deployments
        .iter()
        .filter_map(|deployment| {
            let id = json_u64_field(deployment, "id")?;
            let environment = string_field(deployment, "environment")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "deployment".to_string());
            Some(Deployment {
                id: id.to_string(),
                environment,
                state: "unknown".to_string(),
                description: string_field(deployment, "description").unwrap_or_default(),
                url: None,
                created_at: string_field(deployment, "created_at"),
                updated_at: string_field(deployment, "updated_at"),
                icon_url: deployment
                    .get("creator")
                    .and_then(|creator| string_field(creator, "avatar_url")),
            })
        })
        .collect())
}

fn parse_diff_git_header(line: &str) -> Result<(String, String), String> {
    let input = line
        .strip_prefix("diff --git ")
        .ok_or_else(|| "missing diff header".to_string())?;
    let parts = parse_quoted_tokens(input)?;
    if parts.len() != 2 {
        return Err("unexpected diff header format".to_string());
    }
    Ok((parts[0].clone(), parts[1].clone()))
}

fn parse_quoted_tokens(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '\\' => {
                    let next = chars
                        .next()
                        .ok_or_else(|| "unterminated quoted token".to_string())?;
                    token.push(unescape_char(next));
                }
                '"' => in_quotes = false,
                _ => token.push(ch),
            }
            continue;
        }

        match ch {
            '"' => in_quotes = true,
            ' ' => {
                if !token.is_empty() {
                    tokens.push(token);
                    token = String::new();
                }
            }
            _ => token.push(ch),
        }
    }

    if in_quotes {
        return Err("unterminated quoted token".to_string());
    }
    if !token.is_empty() {
        tokens.push(token);
    }

    Ok(tokens)
}

fn parse_patch_path(value: &str) -> Result<String, String> {
    parse_git_path_value(value.trim())
}

fn parse_git_path_value(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.starts_with('"') {
        parse_quoted_tokens(value)?
            .into_iter()
            .next()
            .ok_or_else(|| "missing quoted path".to_string())
    } else {
        Ok(value.to_string())
    }
}

fn strip_diff_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

fn github_actions_job_id(link: &str) -> Option<String> {
    let (_, tail) = link
        .split_once("/jobs/")
        .or_else(|| link.split_once("/job/"))?;
    let id: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn github_actions_run_id(link: &str) -> Option<String> {
    let (_, tail) = link.split_once("/actions/runs/")?;
    let id: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

fn is_rerunnable_failed_check_state(state: CheckState) -> bool {
    matches!(
        state,
        CheckState::Failure | CheckState::TimedOut | CheckState::StartupFailure
    )
}

fn parse_check_state(value: Option<&str>) -> CheckState {
    let normalized = value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_");

    match normalized.as_str() {
        "queued" => CheckState::Queued,
        "in_progress" | "running" => CheckState::InProgress,
        "pending" => CheckState::Pending,
        "requested" => CheckState::Requested,
        "waiting" => CheckState::Waiting,
        "success" | "successful" => CheckState::Success,
        "failure" | "failed" | "error" => CheckState::Failure,
        "cancelled" | "canceled" => CheckState::Cancelled,
        "skipped" | "skipping" => CheckState::Skipped,
        "neutral" => CheckState::Neutral,
        "action_required" => CheckState::ActionRequired,
        "timed_out" | "timedout" => CheckState::TimedOut,
        "startup_failure" => CheckState::StartupFailure,
        "stale" => CheckState::Stale,
        _ => CheckState::Unknown,
    }
}

fn truncate_log_excerpt(log: &str) -> (String, bool) {
    if log.len() <= MAX_CHECK_LOG_BYTES {
        return (log.to_string(), false);
    }

    let mut start = log.len() - MAX_CHECK_LOG_BYTES;
    while !log.is_char_boundary(start) {
        start += 1;
    }
    (log[start..].to_string(), true)
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn json_u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn parse_output_lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn git_error(context: &str, stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        context.to_string()
    } else {
        format!("{context}: {stderr}")
    }
}

fn is_missing_pull_request_error(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("no pull requests found")
        || stderr.contains("could not find pull request")
        || stderr.contains("pull request not found")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn unescape_char(ch: char) -> char {
    match ch {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ChromeConfig, ClaudeConfig, CodexConfig, GcloudConfig, GitConfig, SiloConfig,
    };
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
            git: GitConfig::default(),
            chrome: ChromeConfig::default(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            projects: IndexMap::from_iter([(
                "demo".to_string(),
                ProjectConfig {
                    name: "demo".to_string(),
                    path: "/tmp/demo".to_string(),
                    image: None,
                    remote_url: "git@github.com:example/demo.git".to_string(),
                    target_branch: "main".to_string(),
                    env_files: Vec::new(),
                    gcloud: Default::default(),
                },
            )]),
        };

        let project = config.projects.get("demo").expect("project should exist");
        assert_eq!(project.target_branch, "main");
        assert_eq!(project.path, "/tmp/demo");
    }

    #[test]
    fn parse_diff_separates_remote_and_local_changes() {
        let diff = "\
===REMOTE_DIFF===\n\
diff --git a/src/old.rs b/src/new.rs\n\
similarity index 90%\n\
rename from src/old.rs\n\
rename to src/new.rs\n\
--- a/src/old.rs\n\
+++ b/src/new.rs\n\
@@ -1,2 +1,2 @@\n\
-old\n\
+new\n\
 same\n\
===LOCAL_DIFF===\n\
diff --git a/docs/guide.md b/docs/guide.md\n\
new file mode 100644\n\
--- /dev/null\n\
+++ b/docs/guide.md\n\
@@ -0,0 +1,2 @@\n\
+hello\n\
+world\n\
";

        let parsed = parse_diff(diff).expect("diff should parse");
        assert_eq!(parsed.overview.additions, 3);
        assert_eq!(parsed.overview.deletions, 1);
        assert_eq!(parsed.overview.files_changed, 2);
        assert_eq!(parsed.remote.overview.additions, 1);
        assert_eq!(parsed.remote.overview.deletions, 1);
        assert_eq!(parsed.remote.overview.files_changed, 1);
        assert_eq!(parsed.remote.files[0].status, "renamed");
        assert_eq!(parsed.remote.files[0].path, "src/new.rs");
        assert_eq!(
            parsed.remote.files[0].previous_path.as_deref(),
            Some("src/old.rs")
        );
        assert_eq!(parsed.local.overview.additions, 2);
        assert_eq!(parsed.local.overview.deletions, 0);
        assert_eq!(parsed.local.overview.files_changed, 1);
        assert_eq!(parsed.local.files[0].status, "added");
        assert_eq!(parsed.local.files[0].path, "docs/guide.md");
    }

    #[test]
    fn parse_diff_does_not_set_previous_path_for_modified_files() {
        let diff = "\
===REMOTE_DIFF===\n\
===LOCAL_DIFF===\n\
diff --git a/src/app.rs b/src/app.rs\n\
index 1111111..2222222 100644\n\
--- a/src/app.rs\n\
+++ b/src/app.rs\n\
@@ -1 +1 @@\n\
-old\n\
+new\n\
";

        let parsed = parse_diff(diff).expect("diff should parse");
        assert_eq!(parsed.remote.files.len(), 0);
        assert_eq!(parsed.local.files.len(), 1);
        assert_eq!(parsed.local.files[0].status, "modified");
        assert_eq!(parsed.local.files[0].path, "src/app.rs");
        assert_eq!(parsed.local.files[0].previous_path, None);
    }

    #[test]
    fn parse_pull_requests_prefers_open_and_requires_matching_head_and_base() {
        let stdout = r#"
[
  {
    "number": 14,
    "headRefName": "feature/a",
    "baseRefName": "develop",
    "headRefOid": "abc123",
    "state": "OPEN",
    "mergedAt": null,
    "updatedAt": "2026-03-10T10:00:00Z",
    "url": "https://github.com/example/repo/pull/14",
    "title": "Wrong base",
    "body": "body 14"
  },
  {
    "number": 15,
    "headRefName": "feature/a",
    "baseRefName": "main",
    "headRefOid": "def456",
    "state": "CLOSED",
    "mergedAt": "2026-03-11T10:00:00Z",
    "updatedAt": "2026-03-11T10:00:00Z",
    "url": "https://github.com/example/repo/pull/15",
    "title": "Merged PR",
    "body": "body 15"
  },
  {
    "number": 16,
    "headRefName": "feature/a",
    "baseRefName": "main",
    "headRefOid": "fedcba",
    "state": "OPEN",
    "mergedAt": null,
    "updatedAt": "2026-03-12T10:00:00Z",
    "url": "https://github.com/example/repo/pull/16",
    "title": "Active PR",
    "body": "body 16"
  }
]
"#;

        let pr = parse_pull_requests(stdout, "feature/a", "main")
            .expect("pull requests should parse")
            .expect("matching pull request should exist");
        assert_eq!(pr.number, 16);
        assert_eq!(pr.head_ref_oid, "fedcba");
        assert_eq!(pr.status, "open");
        assert_eq!(pr.url, "https://github.com/example/repo/pull/16");
        assert_eq!(pr.title, "Active PR");
        assert_eq!(pr.body, "body 16");
    }

    #[test]
    fn parse_pull_requests_returns_latest_non_open_status() {
        let stdout = r#"
[
  {
    "number": 21,
    "headRefName": "feature/a",
    "baseRefName": "main",
    "headRefOid": "aaa111",
    "state": "CLOSED",
    "mergedAt": null,
    "updatedAt": "2026-03-11T10:00:00Z",
    "url": "https://github.com/example/repo/pull/21",
    "title": "Closed PR",
    "body": ""
  },
  {
    "number": 22,
    "headRefName": "feature/a",
    "baseRefName": "main",
    "headRefOid": "bbb222",
    "state": "CLOSED",
    "mergedAt": "2026-03-12T10:00:00Z",
    "updatedAt": "2026-03-12T10:00:00Z",
    "url": "https://github.com/example/repo/pull/22",
    "title": "Merged PR",
    "body": "merged body"
  }
]
"#;

        let pr = parse_pull_requests(stdout, "feature/a", "main")
            .expect("pull requests should parse")
            .expect("matching pull request should exist");
        assert_eq!(pr.number, 22);
        assert_eq!(pr.status, "merged");
        assert_eq!(pr.title, "Merged PR");
        assert_eq!(pr.body, "merged body");
    }

    #[test]
    fn tree_dirty_remote_command_checks_index_worktree_and_untracked_files() {
        let command = tree_dirty_remote_command();
        assert!(command.contains("git diff --quiet --ignore-submodules --"));
        assert!(command.contains("git diff --cached --quiet --ignore-submodules --"));
        assert!(command.contains("git ls-files --others --exclude-standard"));
    }

    #[test]
    fn rename_branch_remote_command_validates_and_renames_branch() {
        let command = rename_branch_remote_command("feature/test");
        assert!(command.contains("BRANCH='feature/test'"));
        assert!(command.contains("git check-ref-format --branch \"$BRANCH\" >/dev/null"));
        assert!(command.contains("git branch -m \"$BRANCH\""));
    }

    #[test]
    fn retarget_branch_remote_command_fetches_and_rebases_target_branch() {
        let command = retarget_branch_remote_command("release/2026");
        assert!(command.contains("TARGET_BRANCH='release/2026'"));
        assert!(command.contains("git fetch --quiet origin \"$TARGET_BRANCH\""));
        assert!(command.contains("git rebase \"origin/$TARGET_BRANCH\""));
    }

    #[test]
    fn rerun_failed_checks_remote_command_targets_failed_jobs_only() {
        let command = rerun_failed_checks_remote_command("123456789");
        assert_eq!(command, "gh run rerun '123456789' --failed");
    }

    #[test]
    fn trim_branch_input_rejects_empty_values() {
        assert_eq!(
            trim_branch_input("  feature/test  ", "branch").expect("branch should trim"),
            "feature/test"
        );
        assert_eq!(
            trim_branch_input(" \t ", "target branch").unwrap_err(),
            "target branch must not be empty"
        );
    }

    #[test]
    fn parse_tree_dirty_output_accepts_boolean_output() {
        assert!(parse_tree_dirty_output("true\n").expect("true should parse"));
        assert!(!parse_tree_dirty_output("false\n").expect("false should parse"));
        assert!(parse_tree_dirty_output("maybe\n")
            .expect_err("unexpected output should fail")
            .contains("unexpected output"));
    }

    #[test]
    fn github_actions_job_id_extracts_job_segment() {
        assert_eq!(
            github_actions_job_id("https://github.com/example/repo/actions/runs/1/job/987654321"),
            Some("987654321".to_string())
        );
        assert_eq!(github_actions_job_id("https://example.com"), None);
    }

    #[test]
    fn github_actions_run_id_extracts_run_segment() {
        assert_eq!(
            github_actions_run_id(
                "https://github.com/example/repo/actions/runs/123456789/job/987654321"
            ),
            Some("123456789".to_string())
        );
        assert_eq!(
            github_actions_run_id("https://github.com/example/repo/checks?check_run_id=42"),
            None
        );
    }

    #[test]
    fn is_rerunnable_failed_check_state_matches_strict_failed_states() {
        assert!(is_rerunnable_failed_check_state(CheckState::Failure));
        assert!(is_rerunnable_failed_check_state(CheckState::TimedOut));
        assert!(is_rerunnable_failed_check_state(CheckState::StartupFailure));
        assert!(!is_rerunnable_failed_check_state(CheckState::Cancelled));
        assert!(!is_rerunnable_failed_check_state(CheckState::Success));
        assert!(!is_rerunnable_failed_check_state(CheckState::Neutral));
        assert!(!is_rerunnable_failed_check_state(CheckState::Skipped));
    }

    #[test]
    fn truncate_log_excerpt_marks_large_logs() {
        let log = "a".repeat(MAX_CHECK_LOG_BYTES + 32);
        let (excerpt, truncated) = truncate_log_excerpt(&log);
        assert!(truncated);
        assert_eq!(excerpt.len(), MAX_CHECK_LOG_BYTES);
    }

    #[test]
    fn parse_check_state_normalizes_known_values() {
        assert_eq!(parse_check_state(Some("SUCCESS")), CheckState::Success);
        assert_eq!(
            parse_check_state(Some("in_progress")),
            CheckState::InProgress
        );
        assert_eq!(
            parse_check_state(Some("action required")),
            CheckState::ActionRequired
        );
        assert_eq!(parse_check_state(Some("skipping")), CheckState::Skipped);
        assert_eq!(
            parse_check_state(Some("weird-new-state")),
            CheckState::Unknown
        );
        assert_eq!(parse_check_state(None), CheckState::Unknown);
    }
}
