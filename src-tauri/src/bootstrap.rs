use crate::config::{ConfigStore, ProjectConfig};
use crate::remote::{
    remote_command_error, run_remote_command, run_remote_command_with_stdin,
    run_terminal_user_command, shell_quote, REMOTE_WORKSPACE_OBSERVER_BIN, TERMINAL_WORKSPACE_DIR,
};
use crate::workspaces::{self, Workspace, WorkspaceLookup};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const REMOTE_BOOTSTRAP_STATE_FILE: &str = "/home/silo/.silo/workspace-bootstrap-state";
const REMOTE_BOOTSTRAP_LOCK_DIR: &str = "/home/silo/.silo/workspace-bootstrap.lock";
const REMOTE_CREDENTIALS_FILE: &str = "/home/silo/.silo/credentials.sh";
const REMOTE_WORKSPACE_OBSERVER_PIDFILE: &str = "/home/silo/.silo/workspace-observer/daemon.pid";
const REMOTE_WORKSPACE_OBSERVER_SHELL_FILE: &str = "/home/silo/.silo/workspace-observer-shell.sh";
const WORKSPACE_BOOTSTRAP_VERSION: &str = "11";
const STARTUP_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const INSTANCE_RUNNING_POLL_ATTEMPTS: usize = 180;
const SSH_READY_POLL_ATTEMPTS: usize = 120;
const BOOTSTRAP_RETRY_ATTEMPTS: usize = 60;
const OBSERVER_READY_POLL_ATTEMPTS: usize = 30;
const OBSERVER_HEARTBEAT_STALE_AFTER_SECS: i64 = 45;
const WORKSPACE_OBSERVER_BIN_BYTES: &[u8] = include_bytes!(concat!(
    env!("OUT_DIR"),
    "/workspace-observer-x86_64-unknown-linux-musl"
));
static WORKSPACE_STARTUP_RECONCILE_IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone)]
struct WorkspaceBootstrap {
    remote_url: String,
    target_branch: String,
    workspace_branch: Option<String>,
    gh_username: String,
    gh_token: String,
    codex_token: String,
    claude_token: String,
    git_user_name: String,
    git_user_email: String,
    env_files: Vec<BootstrapEnvFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BootstrapEnvFile {
    pub(crate) relative_path: String,
    pub(crate) contents_base64: String,
    pub(crate) contents_sha256: String,
}

#[derive(Default)]
struct LifecycleReporter {
    phase: Option<String>,
    detail: Option<String>,
    last_error: Option<String>,
}

async fn bootstrap_workspace(lookup: &WorkspaceLookup) -> Result<(), String> {
    let started = Instant::now();
    log::info!("bootstrapping workspace {}", lookup.workspace.name());
    let bootstrap = workspace_bootstrap(lookup)?;
    let bootstrap_signature = workspace_bootstrap_signature(lookup.workspace.name(), &bootstrap);
    let script = workspace_bootstrap_script(lookup, &bootstrap);
    let result = run_remote_command_with_stdin(
        lookup,
        &run_terminal_user_command("bash -se"),
        script.into_bytes(),
    )
    .await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to bootstrap workspace",
            &result.stderr,
        ));
    }

    persist_workspace_bootstrap_state(lookup, &bootstrap_signature).await?;
    log::info!(
        "workspace {} bootstrap completed duration_ms={}",
        lookup.workspace.name(),
        started.elapsed().as_millis()
    );

    Ok(())
}

pub(crate) fn start_template_bootstrap(workspace: String) {
    start_workspace_startup_reconcile(workspace);
}

pub(crate) fn start_workspace_startup_reconcile_if_needed(workspace: Workspace) {
    if workspace.should_reconcile_startup() {
        start_workspace_startup_reconcile(workspace.name().to_string());
    }
}

pub(crate) fn start_workspace_startup_reconcile(workspace: String) {
    let inserted = WORKSPACE_STARTUP_RECONCILE_IN_FLIGHT
        .lock()
        .map(|mut in_flight| in_flight.insert(workspace.clone()))
        .unwrap_or(false);
    if !inserted {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let result = reconcile_workspace_startup(&workspace).await;
        if let Err(error) = result {
            if let Err(update_error) = workspaces::set_workspace_lifecycle(
                &workspace,
                "failed",
                Some("Workspace startup failed"),
                Some(&error),
            )
            .await
            {
                log::warn!(
                    "failed to publish startup failure lifecycle for workspace {}: {}",
                    workspace,
                    update_error
                );
            }
            log::warn!(
                "background workspace startup reconcile failed for workspace {}: {}",
                workspace,
                error
            );
        } else {
            log::info!(
                "background workspace startup reconcile completed for workspace {}",
                workspace
            );
        }

        if let Ok(mut in_flight) = WORKSPACE_STARTUP_RECONCILE_IN_FLIGHT.lock() {
            in_flight.remove(&workspace);
        }
    });
}

pub(crate) async fn wait_for_template_bootstrap(workspace: &str) -> Result<(), String> {
    for _ in 0..INSTANCE_RUNNING_POLL_ATTEMPTS {
        let lookup = workspaces::find_workspace(workspace).await?;
        if !lookup.workspace.is_template() {
            return Err(format!("workspace {workspace} is not a template workspace"));
        }

        if lookup.workspace.is_ready() {
            return Ok(());
        }

        start_workspace_startup_reconcile_if_needed(lookup.workspace.clone());

        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "template workspace {workspace} did not finish bootstrapping in time"
    ))
}

pub(crate) async fn clear_template_runtime_state(workspace: &str) -> Result<(), String> {
    let lookup = workspaces::find_workspace(workspace).await?;
    if !lookup.workspace.is_template() {
        return Err(format!("workspace {workspace} is not a template workspace"));
    }

    let command = clear_template_runtime_state_command();
    let result = run_remote_command(&lookup, &run_terminal_user_command(&command)).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to clear template runtime state",
            &result.stderr,
        ));
    }

    Ok(())
}

pub(crate) fn should_retry_template_bootstrap(error: &str) -> bool {
    if is_retryable_terminal_transport_error(error) {
        return true;
    }

    let lower = error.to_ascii_lowercase();
    ["system is booting up", "not permitted to log in yet"]
        .iter()
        .any(|needle| lower.contains(needle))
}

pub(crate) fn is_retryable_terminal_transport_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "can't assign requested address",
        "broken pipe",
        "connection refused",
        "connection reset",
        "connection reset by peer",
        "connection timed out",
        "connection closed",
        "connection lost",
        "network is unreachable",
        "operation timed out",
        "port 22",
        "software caused connection abort",
        "timed out",
        "transport endpoint is not connected",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn reconcile_workspace_startup(workspace: &str) -> Result<(), String> {
    let started = Instant::now();
    let mut reporter = LifecycleReporter::default();
    let lookup = wait_for_workspace_running(workspace).await?;

    reporter
        .publish(
            &lookup,
            "waiting_for_ssh",
            Some("Waiting for the VM to accept SSH connections"),
            None,
        )
        .await?;
    wait_for_workspace_ssh(&lookup).await?;

    reporter
        .publish(
            &lookup,
            "bootstrapping",
            Some("Preparing repository, credentials, and tools"),
            None,
        )
        .await?;
    bootstrap_workspace_until_ready(&lookup).await?;

    reporter
        .publish(
            &lookup,
            "waiting_for_observer",
            Some("Waiting for workspace services to come online"),
            None,
        )
        .await?;
    ensure_workspace_observer_running(&lookup).await?;
    wait_for_workspace_observer(workspace).await?;

    reporter.publish(&lookup, "ready", None, None).await?;
    log::info!(
        "workspace {} startup lifecycle reached ready duration_ms={}",
        workspace,
        started.elapsed().as_millis()
    );
    Ok(())
}

async fn wait_for_workspace_running(workspace: &str) -> Result<WorkspaceLookup, String> {
    for attempt in 0..INSTANCE_RUNNING_POLL_ATTEMPTS {
        let lookup = match workspaces::find_workspace(workspace).await {
            Ok(lookup) => lookup,
            Err(error) => {
                if attempt + 1 == INSTANCE_RUNNING_POLL_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(STARTUP_POLL_INTERVAL);
                continue;
            }
        };
        if lookup.workspace.status() == "RUNNING" {
            return Ok(lookup);
        }
        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace {workspace} did not reach RUNNING after {} seconds",
        INSTANCE_RUNNING_POLL_ATTEMPTS * STARTUP_POLL_INTERVAL.as_secs() as usize
    ))
}

async fn wait_for_workspace_ssh(lookup: &WorkspaceLookup) -> Result<(), String> {
    let started = Instant::now();
    for attempt in 0..SSH_READY_POLL_ATTEMPTS {
        let result = run_remote_command(lookup, &run_terminal_user_command("true")).await;
        match result {
            Ok(result) if result.success => {
                log::info!(
                    "workspace {} ssh probe succeeded attempt={} elapsed_ms={}",
                    lookup.workspace.name(),
                    attempt + 1,
                    started.elapsed().as_millis()
                );
                return Ok(());
            }
            Ok(result) => {
                let error = result.stderr.trim().to_string();
                if attempt + 1 == SSH_READY_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(remote_command_error(
                        "failed to verify workspace ssh readiness",
                        &result.stderr,
                    ));
                }
            }
            Err(error) => {
                if attempt + 1 == SSH_READY_POLL_ATTEMPTS
                    || !should_retry_template_bootstrap(&error)
                {
                    return Err(error);
                }
            }
        }

        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace {} did not become ssh-ready after {} seconds",
        lookup.workspace.name(),
        SSH_READY_POLL_ATTEMPTS * STARTUP_POLL_INTERVAL.as_secs() as usize
    ))
}

async fn bootstrap_workspace_until_ready(lookup: &WorkspaceLookup) -> Result<(), String> {
    for attempt in 0..BOOTSTRAP_RETRY_ATTEMPTS {
        match bootstrap_workspace(lookup).await {
            Ok(()) => return Ok(()),
            Err(error) if should_retry_template_bootstrap(&error) => {
                if attempt + 1 == BOOTSTRAP_RETRY_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(STARTUP_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }

    Err(format!(
        "workspace {} did not finish bootstrapping in time",
        lookup.workspace.name()
    ))
}

async fn ensure_workspace_observer_running(lookup: &WorkspaceLookup) -> Result<(), String> {
    let command = ensure_workspace_observer_running_command(lookup);
    let result = run_remote_command(lookup, &run_terminal_user_command(&command)).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to start workspace observer",
            &result.stderr,
        ));
    }
    Ok(())
}

async fn wait_for_workspace_observer(workspace: &str) -> Result<(), String> {
    for attempt in 0..OBSERVER_READY_POLL_ATTEMPTS {
        let lookup = workspaces::find_workspace(workspace).await?;
        if observer_heartbeat_is_fresh(&lookup.workspace) {
            return Ok(());
        }
        if attempt + 1 == OBSERVER_READY_POLL_ATTEMPTS {
            break;
        }
        std::thread::sleep(STARTUP_POLL_INTERVAL);
    }

    Err(format!(
        "workspace observer for {workspace} did not publish a recent heartbeat"
    ))
}

fn observer_heartbeat_is_fresh(workspace: &Workspace) -> bool {
    let Some(heartbeat) = workspace.observer_heartbeat_at() else {
        return false;
    };
    let Ok(heartbeat) = OffsetDateTime::parse(heartbeat, &Rfc3339) else {
        return false;
    };

    OffsetDateTime::now_utc() - heartbeat
        <= time::Duration::seconds(OBSERVER_HEARTBEAT_STALE_AFTER_SECS)
}

impl LifecycleReporter {
    async fn publish(
        &mut self,
        lookup: &WorkspaceLookup,
        phase: &str,
        detail: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<(), String> {
        let next_phase = Some(phase.to_string());
        let next_detail = detail.map(|value| value.to_string());
        let next_error = last_error.map(|value| value.to_string());
        if self.phase == next_phase && self.detail == next_detail && self.last_error == next_error {
            return Ok(());
        }

        workspaces::set_workspace_lifecycle_in_lookup(lookup.clone(), phase, detail, last_error)
            .await?;
        self.phase = next_phase;
        self.detail = next_detail;
        self.last_error = next_error;
        Ok(())
    }
}

fn workspace_bootstrap(lookup: &WorkspaceLookup) -> Result<WorkspaceBootstrap, String> {
    let project_name = lookup.workspace.project().ok_or_else(|| {
        format!(
            "workspace {} is missing a project label",
            lookup.workspace.name()
        )
    })?;
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let project = config.projects.get(project_name).ok_or_else(|| {
        format!(
            "project not found for workspace {}: {project_name}",
            lookup.workspace.name()
        )
    })?;

    if project.remote_url.trim().is_empty() {
        return Err(format!("project {project_name} is missing remote_url"));
    }

    let target_branch = if lookup.workspace.is_template() {
        project.target_branch.trim().to_string()
    } else {
        lookup
            .workspace
            .target_branch()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(project.target_branch.as_str())
            .trim()
            .to_string()
    };
    if target_branch.is_empty() {
        return Err(format!("project {project_name} is missing a target branch"));
    }

    let workspace_branch = if lookup.workspace.is_template() {
        None
    } else {
        let branch = lookup
            .workspace
            .branch_name()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if branch.is_empty() {
            return Err(format!(
                "workspace {} is missing branch metadata",
                lookup.workspace.name()
            ));
        }
        Some(branch)
    };

    Ok(WorkspaceBootstrap {
        remote_url: project.remote_url.clone(),
        target_branch,
        workspace_branch,
        gh_username: config.git.gh_username.clone(),
        gh_token: config.git.gh_token.clone(),
        codex_token: config.codex.token.clone(),
        claude_token: config.claude.token.clone(),
        git_user_name: config.git.user_name.clone(),
        git_user_email: config.git.user_email.clone(),
        env_files: load_bootstrap_env_files(project_name, project),
    })
}

fn workspace_bootstrap_script(lookup: &WorkspaceLookup, bootstrap: &WorkspaceBootstrap) -> String {
    let codex_auth_json = codex_auth_json(&bootstrap.codex_token);
    let codex_config_toml = codex_config_toml();
    let claude_settings_json = claude_settings_json();
    let claude_state_json = claude_state_json();
    let gh_hosts_yml = gh_hosts_yml(&bootstrap.gh_username, &bootstrap.gh_token);
    let bootstrap_signature = workspace_bootstrap_signature(lookup.workspace.name(), bootstrap);
    let observer_install = workspace_observer_install_script(lookup);
    let env_file_sync = if lookup.workspace.is_template() {
        workspace_env_file_sync_script(&bootstrap.env_files)
    } else {
        String::new()
    };
    let credentials_lines = [
        format!("export GH_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!("export GITHUB_TOKEN={}", shell_quote(&bootstrap.gh_token)),
        format!(
            "export CLAUDE_CODE_OAUTH_TOKEN={}",
            shell_quote(&bootstrap.claude_token)
        ),
        "export GIT_ASKPASS=\"$HOME/.silo/git-askpass.sh\"".to_string(),
        "export GIT_TERMINAL_PROMPT=0".to_string(),
        "unset ANTHROPIC_API_KEY".to_string(),
        "unset ANTHROPIC_AUTH_TOKEN".to_string(),
        "unset ANTHROPIC_BASE_URL".to_string(),
        "unset CLAUDE_API_KEY".to_string(),
        "unset CLAUDE_CODE_USE_BEDROCK".to_string(),
        "unset CLAUDE_CODE_USE_VERTEX".to_string(),
        "unset AWS_BEARER_TOKEN_BEDROCK".to_string(),
        "unset VERTEX_REGION_CLAUDE_CODE".to_string(),
        format!(
            "if [[ -f {} ]]; then source {}; fi",
            shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE),
            shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE)
        ),
    ]
    .join("\n");
    let git_clone_target_branch = bootstrap_git_command(
        "clone --branch \"$TARGET_BRANCH\" \"$REMOTE_URL\" \"$WORKSPACE_DIR\"",
    );
    let git_fetch_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\"");
    let git_pull_target_branch =
        bootstrap_git_command("-C \"$WORKSPACE_DIR\" pull --ff-only origin \"$TARGET_BRANCH\"");
    let branch_setup = if lookup.workspace.is_template() {
        format!(
            "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nelse\n  git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n  {git_fetch_target_branch}\n  git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" reset --hard \"origin/$TARGET_BRANCH\"\n  git -C \"$WORKSPACE_DIR\" clean -fd\nfi",
            git_clone_target_branch = git_clone_target_branch,
            git_fetch_target_branch = git_fetch_target_branch,
        )
    } else {
        format!(
            "if [ ! -d \"$WORKSPACE_DIR/.git\" ]; then\n  rm -rf \"$WORKSPACE_DIR\"\n  {git_clone_target_branch}\nfi\n\
git -C \"$WORKSPACE_DIR\" remote set-url origin \"$REMOTE_URL\"\n\
{git_fetch_target_branch}\n\
git -C \"$WORKSPACE_DIR\" checkout \"$TARGET_BRANCH\"\n\
{git_pull_target_branch}\n\
if git -C \"$WORKSPACE_DIR\" show-ref --verify --quiet \"refs/heads/$WORKSPACE_BRANCH\"; then\n  git -C \"$WORKSPACE_DIR\" checkout \"$WORKSPACE_BRANCH\"\nelse\n  git -C \"$WORKSPACE_DIR\" checkout -b \"$WORKSPACE_BRANCH\" \"$TARGET_BRANCH\"\nfi",
            git_clone_target_branch = git_clone_target_branch,
            git_fetch_target_branch = git_fetch_target_branch,
            git_pull_target_branch = git_pull_target_branch,
        )
    };

    format!(
        "set -euo pipefail\n\
WORKSPACE_DIR={workspace_dir}\n\
REMOTE_URL={remote_url}\n\
TARGET_BRANCH={target_branch}\n\
WORKSPACE_BRANCH={workspace_branch}\n\
GIT_USER_NAME={git_user_name}\n\
GIT_USER_EMAIL={git_user_email}\n\
mkdir -p \"$HOME/.silo\"\n\
chmod 700 \"$HOME/.silo\"\n\
LOCK_DIR={lock_dir}\n\
ASKPASS_PATH=\"$HOME/.silo/git-askpass.sh\"\n\
cleanup() {{\n\
  rm -rf \"$LOCK_DIR\"\n\
}}\n\
for _ in $(seq 1 60); do\n\
  if mkdir \"$LOCK_DIR\" 2>/dev/null; then\n\
    trap cleanup EXIT\n\
    break\n\
  fi\n\
  sleep 1\n\
done\n\
if [ ! -d \"$LOCK_DIR\" ]; then\n\
  echo 'timed out waiting for workspace bootstrap lock' >&2\n\
  exit 1\n\
fi\n\
BOOT_ID=\"$(cat /proc/sys/kernel/random/boot_id)\"\n\
STATE_PATH={state_path}\n\
SIGNATURE={signature}\n\
if [ -f \"$STATE_PATH\" ]; then\n\
  CURRENT_BOOT_ID=\"$(sed -n '1p' \"$STATE_PATH\")\"\n\
  CURRENT_SIGNATURE=\"$(sed -n '2,$p' \"$STATE_PATH\")\"\n\
  if [ \"$CURRENT_BOOT_ID\" = \"$BOOT_ID\" ] && [ \"$CURRENT_SIGNATURE\" = \"$SIGNATURE\" ]; then\n\
    exit 0\n\
  fi\n\
fi\n\
cat > \"$ASKPASS_PATH\" <<'EOF_GIT_ASKPASS'\n\
#!/bin/sh\n\
case \"$1\" in\n\
  *Username*) printf '%s\\n' 'x-access-token' ;;\n\
  *Password*) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
  *) printf '%s\\n' \"${{GH_TOKEN:-}}\" ;;\n\
esac\n\
EOF_GIT_ASKPASS\n\
chmod 700 \"$ASKPASS_PATH\"\n\
cat > {credentials_path} <<'EOF'\n\
{credentials_lines}\n\
EOF\n\
chmod 600 {credentials_path}\n\
. {credentials_path}\n\
for rc in \"$HOME/.zshrc\" \"$HOME/.bashrc\"; do\n\
  touch \"$rc\"\n\
  if ! grep -Fqx '[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"' \"$rc\"; then\n\
    printf '\\n[[ -f \"$HOME/.silo/credentials.sh\" ]] && source \"$HOME/.silo/credentials.sh\"\\n' >> \"$rc\"\n\
  fi\n\
done\n\
mkdir -p \"$HOME/.config/gh\"\n\
printf '%s\\n' {gh_hosts_yml} > \"$HOME/.config/gh/hosts.yml\"\n\
chmod 700 \"$HOME/.config\" \"$HOME/.config/gh\"\n\
chmod 600 \"$HOME/.config/gh/hosts.yml\"\n\
mkdir -p \"$HOME/.codex\" \"$HOME/.claude\"\n\
printf '%s\\n' {codex_auth_json} > \"$HOME/.codex/auth.json\"\n\
printf '%s\\n' {codex_config_toml} > \"$HOME/.codex/config.toml\"\n\
printf '%s\\n' {claude_settings_json} > \"$HOME/.claude/settings.json\"\n\
printf '%s\\n' {claude_state_json} > \"$HOME/.claude.json\"\n\
chmod 700 \"$HOME/.codex\" \"$HOME/.claude\"\n\
chmod 600 \"$HOME/.codex/auth.json\" \"$HOME/.codex/config.toml\" \"$HOME/.claude/settings.json\" \"$HOME/.claude.json\"\n\
rm -f \"$HOME/.gitconfig.lock\"\n\
if [ -n \"$GIT_USER_NAME\" ] && [ \"$(git config --global --get user.name || true)\" != \"$GIT_USER_NAME\" ]; then\n\
  git config --global user.name \"$GIT_USER_NAME\"\n\
fi\n\
if [ -n \"$GIT_USER_EMAIL\" ] && [ \"$(git config --global --get user.email || true)\" != \"$GIT_USER_EMAIL\" ]; then\n\
  git config --global user.email \"$GIT_USER_EMAIL\"\n\
fi\n\
if ! git config --global --get-all safe.directory 2>/dev/null | grep -Fxq \"$WORKSPACE_DIR\"; then\n\
  git config --global --add safe.directory \"$WORKSPACE_DIR\"\n\
fi\n\
{branch_setup}\n\
{env_file_sync}\n\
{observer_install}",
        workspace_dir = shell_quote(TERMINAL_WORKSPACE_DIR),
        remote_url = shell_quote(&bootstrap.remote_url),
        target_branch = shell_quote(&bootstrap.target_branch),
        workspace_branch = shell_quote(bootstrap.workspace_branch.as_deref().unwrap_or("")),
        git_user_name = shell_quote(&bootstrap.git_user_name),
        git_user_email = shell_quote(&bootstrap.git_user_email),
        lock_dir = shell_quote(REMOTE_BOOTSTRAP_LOCK_DIR),
        state_path = shell_quote(REMOTE_BOOTSTRAP_STATE_FILE),
        signature = shell_quote(&bootstrap_signature),
        credentials_path = shell_quote(REMOTE_CREDENTIALS_FILE),
        credentials_lines = credentials_lines,
        gh_hosts_yml = shell_quote(&gh_hosts_yml),
        codex_auth_json = shell_quote(&codex_auth_json),
        codex_config_toml = shell_quote(&codex_config_toml),
        claude_settings_json = shell_quote(&claude_settings_json),
        claude_state_json = shell_quote(&claude_state_json),
        branch_setup = branch_setup,
        env_file_sync = env_file_sync,
        observer_install = observer_install,
    )
}

pub(crate) fn bootstrap_git_command(command: &str) -> String {
    format!(
        "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git {command}"
    )
}

fn workspace_bootstrap_signature(workspace_name: &str, bootstrap: &WorkspaceBootstrap) -> String {
    format!(
        "version={}\nworkspace={}\nremote_url={}\ntarget_branch={}\nworkspace_branch={}\ngh_username={}\ngh_token={}\ncodex_token={}\nclaude_token={}\ngit_user_name={}\ngit_user_email={}\nenv_files={}\nobserver_sources={}",
        WORKSPACE_BOOTSTRAP_VERSION,
        workspace_name,
        bootstrap.remote_url,
        bootstrap.target_branch,
        bootstrap.workspace_branch.as_deref().unwrap_or(""),
        bootstrap.gh_username,
        bootstrap.gh_token,
        bootstrap.codex_token,
        bootstrap.claude_token,
        bootstrap.git_user_name,
        bootstrap.git_user_email,
        bootstrap_env_files_signature(&bootstrap.env_files),
        workspace_observer_source_signature(),
    )
}

pub(crate) fn load_bootstrap_env_files(
    project_name: &str,
    project: &ProjectConfig,
) -> Vec<BootstrapEnvFile> {
    let project_root = Path::new(&project.path);
    let mut env_files = Vec::new();
    let mut seen = HashSet::new();

    for configured_path in &project.env_files {
        let Some(relative_path) = normalize_workspace_relative_path(configured_path) else {
            log::warn!(
                "skipping invalid env file path for project {}: {}",
                project_name,
                configured_path
            );
            continue;
        };

        if !seen.insert(relative_path.clone()) {
            continue;
        }

        let local_path = project_root.join(Path::new(&relative_path));
        let Ok(contents) = fs::read(&local_path) else {
            log::warn!(
                "skipping missing or unreadable env file for project {}: {}",
                project_name,
                local_path.display()
            );
            continue;
        };

        let contents_sha256 = hex_sha256(&contents);
        env_files.push(BootstrapEnvFile {
            relative_path,
            contents_base64: BASE64_STANDARD.encode(contents),
            contents_sha256,
        });
    }

    env_files
}

pub(crate) fn normalize_workspace_relative_path(path: &str) -> Option<String> {
    let mut normalized = String::new();

    for component in Path::new(path).components() {
        let Component::Normal(value) = component else {
            return None;
        };
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(value.to_str()?);
    }

    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn bootstrap_env_files_signature(env_files: &[BootstrapEnvFile]) -> String {
    env_files
        .iter()
        .map(|env_file| format!("{}:{}", env_file.relative_path, env_file.contents_sha256))
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn workspace_env_file_sync_script(env_files: &[BootstrapEnvFile]) -> String {
    if env_files.is_empty() {
        return String::new();
    }

    let mut script = String::from("# sync project env files into the template workspace\n");

    for (index, env_file) in env_files.iter().enumerate() {
        if let Some((parent_dir, _)) = env_file.relative_path.rsplit_once('/') {
            script.push_str(&format!(
                "mkdir -p {}\n",
                shell_quote(&format!("{TERMINAL_WORKSPACE_DIR}/{parent_dir}"))
            ));
        }

        let target_path = format!("{TERMINAL_WORKSPACE_DIR}/{}", env_file.relative_path);
        script.push_str(&format!(
            "cat <<'EOF_ENV_{index}' | base64 --decode > {target_path}\n{contents}\nEOF_ENV_{index}\nchmod 600 {target_path}\n",
            target_path = shell_quote(&target_path),
            contents = env_file.contents_base64,
        ));
    }

    script
}

fn workspace_observer_install_script(lookup: &WorkspaceLookup) -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN);
    let pidfile = shell_quote(REMOTE_WORKSPACE_OBSERVER_PIDFILE);
    let shell_path = shell_quote(REMOTE_WORKSPACE_OBSERVER_SHELL_FILE);
    let encoded_binary = BASE64_STANDARD.encode(WORKSPACE_OBSERVER_BIN_BYTES);
    let shell_script = workspace_observer_shell_script();
    let encoded_shell = BASE64_STANDARD.encode(shell_script.as_bytes());

    let mut script =
        "install -d -m 0700 \"$HOME/.silo\" \"$HOME/.silo/bin\" \"$HOME/.silo/workspace-observer\"\n"
            .to_string();
    script.push_str(&format!(
        "cat <<'EOF_OBSERVER_BIN' | base64 --decode > {bin_path}\n{encoded_binary}\nEOF_OBSERVER_BIN\n",
    ));
    script.push_str(&format!("chmod 0755 {bin_path}\n"));
    script.push_str(&format!(
        "cat <<'EOF_OBSERVER_SHELL' | base64 --decode > {shell_path}\n{encoded_shell}\nEOF_OBSERVER_SHELL\n",
    ));
    script.push_str(&format!("chmod 0755 {shell_path}\n"));
    script.push_str(&format!(
        "if [ -f {pidfile} ]; then kill \"$(cat {pidfile})\" 2>/dev/null || true; rm -f {pidfile}; fi\n",
    ));
    script.push_str(&format!(
        "nohup {bin_path} daemon --instance {instance} --project {project} --zone {zone} >/dev/null 2>&1 < /dev/null &\n",
        instance = shell_quote(lookup.workspace.name()),
        project = shell_quote(&lookup.gcloud_project),
        zone = shell_quote(lookup.workspace.zone()),
    ));
    script
}

fn ensure_workspace_observer_running_command(lookup: &WorkspaceLookup) -> String {
    let bin_path = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN);
    let pidfile = shell_quote(REMOTE_WORKSPACE_OBSERVER_PIDFILE);
    let install_script = workspace_observer_install_script(lookup);
    format!(
        "if [ -x {bin_path} ] && [ -f {pidfile} ]; then\n\
  PID=\"$(cat {pidfile})\"\n\
  if [ -n \"$PID\" ] && kill -0 \"$PID\" 2>/dev/null; then\n\
    exit 0\n\
  fi\n\
fi\n\
{install_script}",
        bin_path = bin_path,
        pidfile = pidfile,
        install_script = install_script,
    )
}

fn workspace_observer_source_signature() -> String {
    let mut hasher = Sha256::new();
    hasher.update(WORKSPACE_OBSERVER_BIN_BYTES);
    hasher.update(workspace_observer_shell_script().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn workspace_observer_shell_script() -> String {
    format!(
        "export SILO_WORKSPACE_OBSERVER_BIN={observer_bin}\n\
_silo_observer_emit() {{\n\
  [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
  [ -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ] || return 0\n\
  SILO_OBSERVER_HOOK=1 \"$SILO_WORKSPACE_OBSERVER_BIN\" emit \"$@\" >/dev/null 2>&1 || true\n\
}}\n\
_silo_observer_wrap_assistant() {{\n\
  local provider=\"$1\"\n\
  shift\n\
  if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ]; then\n\
    command \"$@\"\n\
    return\n\
  fi\n\
  command \"$SILO_WORKSPACE_OBSERVER_BIN\" assistant-proxy --provider \"$provider\" -- \"$@\"\n\
}}\n\
codex() {{\n\
  _silo_observer_wrap_assistant codex codex \"$@\"\n\
}}\n\
claude() {{\n\
  _silo_observer_wrap_assistant claude claude --dangerously-skip-permissions \"$@\"\n\
}}\n\
cc() {{\n\
  claude \"$@\"\n\
}}\n\
silo() {{\n\
  local provider=\"${{1:-}}\"\n\
  shift || true\n\
  case \"$provider\" in\n\
    codex)\n\
      if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ]; then\n\
        command codex \"$@\"\n\
        return\n\
      fi\n\
      command \"$SILO_WORKSPACE_OBSERVER_BIN\" assistant-proxy --provider codex --initial-prompt-argv -- codex \"$@\"\n\
      ;;\n\
    claude)\n\
      if [ -z \"${{ZMX_SESSION:-}}\" ] || [ ! -x \"${{SILO_WORKSPACE_OBSERVER_BIN:-}}\" ]; then\n\
        IS_SANDBOX=1 command claude --dangerously-skip-permissions \"$@\"\n\
        return\n\
      fi\n\
      IS_SANDBOX=1 command \"$SILO_WORKSPACE_OBSERVER_BIN\" assistant-proxy --provider claude --initial-prompt-argv -- claude --dangerously-skip-permissions \"$@\"\n\
      ;;\n\
    *)\n\
      printf 'unsupported silo assistant: %s\\n' \"$provider\" >&2\n\
      return 1\n\
      ;;\n\
  esac\n\
}}\n\
case $- in\n\
  *i*) ;;\n\
  *) return 0 2>/dev/null || exit 0 ;;\n\
esac\n\
if [ -n \"${{ZMX_SESSION:-}}\" ] && [ -z \"${{SILO_OBSERVER_SESSION_REGISTERED:-}}\" ]; then\n\
  export SILO_OBSERVER_SESSION_REGISTERED=1\n\
  _silo_observer_emit --kind shell_session_started --session \"$ZMX_SESSION\"\n\
fi\n\
if [ -n \"${{ZSH_VERSION:-}}\" ]; then\n\
  autoload -Uz add-zsh-hook\n\
  typeset -g SILO_OBSERVER_LAST_COMMAND=\"${{SILO_OBSERVER_LAST_COMMAND:-}}\"\n\
  _silo_observer_preexec() {{\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && return 0\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    SILO_OBSERVER_LAST_COMMAND=\"$1\"\n\
    _silo_observer_emit --kind shell_command_started --session \"$ZMX_SESSION\" --command \"$1\"\n\
  }}\n\
  _silo_observer_precmd() {{\n\
    local exit_code=$?\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && return $exit_code\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return $exit_code\n\
    if [ -n \"${{SILO_OBSERVER_LAST_COMMAND:-}}\" ]; then\n\
      _silo_observer_emit --kind shell_command_finished --session \"$ZMX_SESSION\"\n\
      SILO_OBSERVER_LAST_COMMAND=\"\"\n\
    fi\n\
    return $exit_code\n\
  }}\n\
  _silo_observer_zshexit() {{\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    [ -n \"${{SILO_OBSERVER_SESSION_REGISTERED:-}}\" ] || return 0\n\
    _silo_observer_emit --kind shell_session_exited --session \"$ZMX_SESSION\"\n\
  }}\n\
  case \" ${{preexec_functions[*]:-}} \" in\n\
    *\" _silo_observer_preexec \"*) ;;\n\
    *) add-zsh-hook preexec _silo_observer_preexec ;;\n\
  esac\n\
  case \" ${{precmd_functions[*]:-}} \" in\n\
    *\" _silo_observer_precmd \"*) ;;\n\
    *) add-zsh-hook precmd _silo_observer_precmd ;;\n\
  esac\n\
  case \" ${{zshexit_functions[*]:-}} \" in\n\
    *\" _silo_observer_zshexit \"*) ;;\n\
    *) add-zsh-hook zshexit _silo_observer_zshexit ;;\n\
  esac\n\
elif [ -n \"${{BASH_VERSION:-}}\" ]; then\n\
  SILO_OBSERVER_LAST_COMMAND=\"${{SILO_OBSERVER_LAST_COMMAND:-}}\"\n\
  SILO_OBSERVER_BASH_IN_PROMPT=0\n\
  _silo_observer_bash_preexec() {{\n\
    local exit_code=$?\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && return $exit_code\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return $exit_code\n\
    [ \"${{SILO_OBSERVER_BASH_IN_PROMPT:-0}}\" = \"1\" ] && return $exit_code\n\
    case \"$BASH_COMMAND\" in\n\
      _silo_observer_*|trap*|history*|\"PROMPT_COMMAND=\"*) return $exit_code ;;\n\
    esac\n\
    if [ -n \"${{SILO_OBSERVER_LAST_COMMAND:-}}\" ] && [ \"$BASH_COMMAND\" = \"$SILO_OBSERVER_LAST_COMMAND\" ]; then\n\
      return $exit_code\n\
    fi\n\
    SILO_OBSERVER_LAST_COMMAND=\"$BASH_COMMAND\"\n\
    _silo_observer_emit --kind shell_command_started --session \"$ZMX_SESSION\" --command \"$BASH_COMMAND\"\n\
    return $exit_code\n\
  }}\n\
  _silo_observer_bash_precmd() {{\n\
    local exit_code=$?\n\
    SILO_OBSERVER_BASH_IN_PROMPT=1\n\
    [ -n \"${{SILO_OBSERVER_HOOK:-}}\" ] && {{ SILO_OBSERVER_BASH_IN_PROMPT=0; return $exit_code; }}\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || {{ SILO_OBSERVER_BASH_IN_PROMPT=0; return $exit_code; }}\n\
    if [ -n \"${{SILO_OBSERVER_LAST_COMMAND:-}}\" ]; then\n\
      _silo_observer_emit --kind shell_command_finished --session \"$ZMX_SESSION\"\n\
      SILO_OBSERVER_LAST_COMMAND=\"\"\n\
    fi\n\
    SILO_OBSERVER_BASH_IN_PROMPT=0\n\
    return $exit_code\n\
  }}\n\
  _silo_observer_bash_exit() {{\n\
    [ -n \"${{ZMX_SESSION:-}}\" ] || return 0\n\
    [ -n \"${{SILO_OBSERVER_SESSION_REGISTERED:-}}\" ] || return 0\n\
    _silo_observer_emit --kind shell_session_exited --session \"$ZMX_SESSION\"\n\
  }}\n\
  trap _silo_observer_bash_preexec DEBUG\n\
  case \";${{PROMPT_COMMAND:-}};\" in\n\
    *\";_silo_observer_bash_precmd;\"*) ;;\n\
    *) PROMPT_COMMAND=\"_silo_observer_bash_precmd${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;;\n\
  esac\n\
  trap _silo_observer_bash_exit EXIT\n\
fi\n",
        observer_bin = shell_quote(REMOTE_WORKSPACE_OBSERVER_BIN),
    )
}

pub(crate) fn codex_auth_json(token: &str) -> String {
    let last_refresh = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": serde_json::Value::Null,
        "tokens": {
            "id_token": token,
            "access_token": token,
            "refresh_token": token,
            "account_id": ""
        },
        "last_refresh": last_refresh
    })
    .to_string()
}

fn codex_config_toml() -> String {
    r#"personality = "pragmatic"
model = "gpt-5.4"
model_reasoning_effort = "high"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[projects."/home/silo/workspace"]
trust_level = "trusted"

[notice]
hide_full_access_warning = true
"#
    .to_string()
}

fn claude_settings_json() -> String {
    json!({
        "model": "opus",
        "alwaysThinkingEnabled": true,
        "effortLevel": "high",
        "skipDangerousModePermissionPrompt": true
    })
    .to_string()
}

pub(crate) fn claude_state_json() -> String {
    json!({
        "installMethod": "native",
        "autoUpdates": false,
        "hasCompletedOnboarding": true,
        "projects": {
            TERMINAL_WORKSPACE_DIR: {
                "allowedTools": [],
                "mcpContextUris": [],
                "mcpServers": {},
                "enabledMcpjsonServers": [],
                "disabledMcpjsonServers": [],
                "hasTrustDialogAccepted": true,
                "projectOnboardingSeenCount": 1,
                "hasCompletedProjectOnboarding": true,
                "hasClaudeMdExternalIncludesApproved": false,
                "hasClaudeMdExternalIncludesWarningShown": false
            }
        }
    })
    .to_string()
}

fn gh_hosts_yml(username: &str, token: &str) -> String {
    format!(
        "github.com:\n    user: {username}\n    oauth_token: {token}\n    git_protocol: https\n"
    )
}
async fn persist_workspace_bootstrap_state(
    lookup: &WorkspaceLookup,
    signature: &str,
) -> Result<(), String> {
    let command = persist_workspace_bootstrap_state_command(signature);
    let result = run_remote_command(lookup, &run_terminal_user_command(&command)).await?;
    if !result.success {
        return Err(remote_command_error(
            "failed to persist workspace bootstrap state",
            &result.stderr,
        ));
    }

    Ok(())
}

pub(crate) fn persist_workspace_bootstrap_state_command(signature: &str) -> String {
    let state_path = shell_quote(REMOTE_BOOTSTRAP_STATE_FILE);
    let signature_base64 = shell_quote(&BASE64_STANDARD.encode(signature));
    format!(
        "mkdir -p \"$HOME/.silo\" && BOOT_ID=\"$(cat /proc/sys/kernel/random/boot_id)\" && {{ printf '%s\\n' \"$BOOT_ID\"; printf %s {signature_base64} | base64 --decode; printf '\\n'; }} > {state_path} && chmod 600 {state_path}",
    )
}

pub(crate) fn clear_template_runtime_state_command() -> String {
    "rm -rf \"$HOME/.silo\"".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn workspace_observer_shell_script_emits_shell_session_lifecycle_events() {
        let script = workspace_observer_shell_script();

        assert!(script.contains("--kind shell_session_started"));
        assert!(script.contains("--kind shell_session_exited"));
        assert!(script.contains("SILO_OBSERVER_SESSION_REGISTERED"));
        assert!(script.contains("add-zsh-hook zshexit _silo_observer_zshexit"));
    }

    #[test]
    fn bootstrap_git_command_inlines_auth_env() {
        assert_eq!(
            bootstrap_git_command("-C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""),
            "env GH_TOKEN=\"$GH_TOKEN\" GITHUB_TOKEN=\"$GITHUB_TOKEN\" GIT_ASKPASS=\"$ASKPASS_PATH\" GIT_TERMINAL_PROMPT=0 git -C \"$WORKSPACE_DIR\" fetch origin \"$TARGET_BRANCH\""
        );
    }

    #[test]
    fn bootstrap_retry_detects_broken_pipe() {
        assert!(should_retry_template_bootstrap(
            "failed to write gcloud ssh stdin: Broken pipe (os error 32)"
        ));
    }

    #[test]
    fn transport_retry_detects_local_address_rebind_failure() {
        assert!(is_retryable_terminal_transport_error(
            "Read from remote host 35.245.135.222: Can't assign requested address"
        ));
    }

    #[test]
    fn transport_retry_ignores_missing_remote_session_errors() {
        assert!(!is_retryable_terminal_transport_error(
            "zmx attach: session not found"
        ));
    }

    #[test]
    fn persist_bootstrap_state_command_streams_base64_signature() {
        let command = persist_workspace_bootstrap_state_command("version=10\nworkspace=demo");

        assert!(command.contains("base64 --decode"));
        assert!(command.contains("/home/silo/.silo/workspace-bootstrap-state"));
        assert!(!command.contains("workspace=demo"));
    }

    #[test]
    fn clear_template_runtime_state_command_removes_remote_silo_dir() {
        assert_eq!(
            clear_template_runtime_state_command(),
            "rm -rf \"$HOME/.silo\""
        );
    }

    #[test]
    fn normalize_workspace_relative_path_rejects_non_relative_paths() {
        assert_eq!(
            normalize_workspace_relative_path("apps/web/.env.local"),
            Some("apps/web/.env.local".to_string())
        );
        assert_eq!(
            normalize_workspace_relative_path(".env"),
            Some(".env".to_string())
        );
        assert_eq!(normalize_workspace_relative_path("../.env"), None);
        assert_eq!(normalize_workspace_relative_path("/tmp/.env"), None);
    }

    #[test]
    fn load_bootstrap_env_files_reads_and_hashes_configured_files() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join("apps/web")).expect("nested env dir should exist");
        fs::write(project_root.join(".env.local"), "ROOT=1\n").expect("root env should exist");
        fs::write(project_root.join("apps/web/.env"), "WEB=1\n").expect("nested env should exist");

        let project = ProjectConfig {
            name: "demo".to_string(),
            path: project_root.to_string_lossy().into_owned(),
            image: None,
            remote_url: "git@github.com:example/demo.git".to_string(),
            target_branch: "main".to_string(),
            env_files: vec![
                ".env.local".to_string(),
                "apps/web/.env".to_string(),
                "../ignored".to_string(),
            ],
            gcloud: Default::default(),
        };

        let env_files = load_bootstrap_env_files("demo", &project);

        assert_eq!(env_files.len(), 2);
        assert_eq!(env_files[0].relative_path, ".env.local");
        assert_eq!(env_files[0].contents_sha256, hex_sha256(b"ROOT=1\n"));
        assert_eq!(env_files[1].relative_path, "apps/web/.env");
        assert_eq!(env_files[1].contents_sha256, hex_sha256(b"WEB=1\n"));
    }

    #[test]
    fn workspace_env_file_sync_script_preserves_relative_paths() {
        let script = workspace_env_file_sync_script(&[BootstrapEnvFile {
            relative_path: "apps/web/.env.local".to_string(),
            contents_base64: BASE64_STANDARD.encode("WEB=1\n"),
            contents_sha256: "digest".to_string(),
        }]);

        assert!(script.contains("mkdir -p '/home/silo/workspace/apps/web'"));
        assert!(script.contains("base64 --decode > '/home/silo/workspace/apps/web/.env.local'"));
        assert!(script.contains("chmod 600 '/home/silo/workspace/apps/web/.env.local'"));
    }

    #[test]
    fn codex_auth_json_contains_access_token() {
        let payload = codex_auth_json("codex-token");
        assert!(payload.contains("\"access_token\":\"codex-token\""));
        assert!(payload.contains("\"auth_mode\":\"chatgpt\""));
    }

    #[test]
    fn claude_state_json_marks_workspace_as_trusted_and_onboarded() {
        let payload = claude_state_json();
        assert!(payload.contains(TERMINAL_WORKSPACE_DIR));
        assert!(payload.contains("\"hasTrustDialogAccepted\":true"));
        assert!(payload.contains("\"hasCompletedOnboarding\":true"));
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = format!(
                "silo-bootstrap-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(Path::new(&self.path));
        }
    }
}
