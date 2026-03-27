use std::env;
use std::io;

use crate::args::required_flag_value;
use crate::assistant::{run_assistant_hook, run_assistant_proxy};
use crate::daemon::run_daemon;
use crate::daemon::state::{
    build_published_state, reconcile_sessions, AssistantProvider, ObserverEvent, PublishedSession,
};
use crate::daemon::zmx::list_zmx_sessions;
use crate::files::{
    run_files_directory, run_files_read, run_files_sync_watch_set, run_files_tree,
    run_files_watch_state, run_files_write,
};
use crate::runtime::{load_state, send_event, write_json_stdout, RuntimePaths};

pub(crate) fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first().map(String::as_str) else {
        return Err("missing command".to_string());
    };

    match command {
        "daemon" => run_daemon(&args[1..]),
        "emit" => run_emit(&args[1..]),
        "mark-read" => run_mark_read(&args[1..]),
        "terminals" => run_terminals(),
        "sessions-snapshot" => run_sessions_snapshot(),
        "session-upsert" => run_session_upsert(),
        "session-remove" => run_session_remove(&args[1..]),
        "session-set-active" => run_session_set_active(&args[1..]),
        "session-clear-active" => run_session_clear_active(),
        "assistant-proxy" => run_assistant_proxy(&args[1..]),
        "assistant-hook" => run_assistant_hook(&args[1..]),
        "files-directory" => run_files_directory(&args[1..]),
        "files-tree" => run_files_tree(),
        "files-read" => run_files_read(&args[1..]),
        "files-write" => run_files_write(&args[1..]),
        "files-sync-watch-set" => run_files_sync_watch_set(),
        "files-watch-state" => run_files_watch_state(),
        other => Err(format!("unknown command: {other}")),
    }
}

fn run_emit(args: &[String]) -> Result<(), String> {
    let event = parse_emit_event(args)?;
    send_event(&RuntimePaths::new().fifo, &event)
}

fn run_mark_read(args: &[String]) -> Result<(), String> {
    let session = required_flag_value(args, "--session")?;
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::MarkRead {
            session: session.to_string(),
        },
    )
}

fn run_terminals() -> Result<(), String> {
    let runtime = RuntimePaths::new();
    let mut state = load_state(&runtime.state_file).unwrap_or_default();
    if let Ok(live_sessions) = list_zmx_sessions() {
        reconcile_sessions(&mut state, &live_sessions);
    }
    let published = build_published_state(&state);
    write_json_stdout(&published.terminals)
}

fn run_sessions_snapshot() -> Result<(), String> {
    let runtime = RuntimePaths::new();
    let mut state = load_state(&runtime.state_file).unwrap_or_default();
    if let Ok(live_sessions) = list_zmx_sessions() {
        reconcile_sessions(&mut state, &live_sessions);
    }
    let published = build_published_state(&state);
    write_json_stdout(&published)
}

fn run_session_upsert() -> Result<(), String> {
    let session = serde_json::from_reader::<_, PublishedSession>(io::stdin())
        .map_err(|error| format!("failed to parse session payload: {error}"))?;
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::SessionUpsert { session },
    )
}

fn run_session_remove(args: &[String]) -> Result<(), String> {
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::SessionRemove {
            session_type: required_flag_value(args, "--type")?.to_string(),
            attachment_id: required_flag_value(args, "--attachment-id")?.to_string(),
        },
    )
}

fn run_session_set_active(args: &[String]) -> Result<(), String> {
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::SetActiveSession {
            session_type: required_flag_value(args, "--type")?.to_string(),
            attachment_id: required_flag_value(args, "--attachment-id")?.to_string(),
        },
    )
}

fn run_session_clear_active() -> Result<(), String> {
    send_event(
        &RuntimePaths::new().fifo,
        &ObserverEvent::ClearActiveSession,
    )
}

fn parse_emit_event(args: &[String]) -> Result<ObserverEvent, String> {
    let kind = required_flag_value(args, "--kind")?;
    let session = required_flag_value(args, "--session")?.to_string();

    match kind {
        "shell_session_started" => Ok(ObserverEvent::ShellSessionStarted { session }),
        "shell_session_exited" => Ok(ObserverEvent::ShellSessionExited { session }),
        "shell_command_started" => Ok(ObserverEvent::ShellCommandStarted {
            session,
            command: required_flag_value(args, "--command")?.to_string(),
        }),
        "shell_command_finished" => Ok(ObserverEvent::ShellCommandFinished { session }),
        "assistant_session_started" => Ok(ObserverEvent::AssistantSessionStarted {
            session,
            provider: AssistantProvider::parse(required_flag_value(args, "--provider")?)
                .ok_or_else(|| "invalid assistant provider".to_string())?,
        }),
        "assistant_prompt_submitted" => Ok(ObserverEvent::AssistantPromptSubmitted {
            session,
            provider: AssistantProvider::parse(required_flag_value(args, "--provider")?)
                .ok_or_else(|| "invalid assistant provider".to_string())?,
        }),
        "assistant_turn_completed" => Ok(ObserverEvent::AssistantTurnCompleted {
            session,
            provider: AssistantProvider::parse(required_flag_value(args, "--provider")?)
                .ok_or_else(|| "invalid assistant provider".to_string())?,
        }),
        "mark_read" => Ok(ObserverEvent::MarkRead { session }),
        other => Err(format!("unsupported event kind: {other}")),
    }
}
