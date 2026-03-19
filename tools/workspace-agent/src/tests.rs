use std::collections::BTreeMap;

use time::Duration as TimeDuration;

use crate::assistant::{
    collect_submitted_assistant_prompts, normalize_assistant_input, turn_output_timeout,
    INITIAL_PROMPT_STARTUP_GRACE, SOFT_NEWLINE_SENTINEL, TURN_OUTPUT_IDLE_TIMEOUT,
};
use crate::daemon::state::{
    apply_event, build_published_state, effective_activity_at, parse_timestamp, reconcile_sessions,
    resolve_assistant_provider, sanitize_command_name, should_suspend_for_inactivity_at,
    AssistantProvider, ObserverEvent, ObserverState, PublishedSession, PublishedState,
    SessionState, POLL_MISS_THRESHOLD_LIFECYCLE,
};
use crate::daemon::zmx::{parse_zmx_session, ZmxSession};
use crate::metadata::{
    bool_metadata_value, flat_metadata_items, update_metadata_item,
    TERMINAL_LAST_ACTIVE_METADATA_KEY, TERMINAL_LAST_WORKING_METADATA_KEY,
    TERMINAL_UNREAD_METADATA_KEY, TERMINAL_WORKING_METADATA_KEY,
};

#[test]
fn assistant_input_strips_escape_sequences() {
    assert_eq!(
        normalize_assistant_input("hello\u{001b}[31m world\u{001b}[0m"),
        "hello world"
    );
}

#[test]
fn assistant_input_collects_prompts() {
    let mut buffer = String::new();
    assert_eq!(
        collect_submitted_assistant_prompts(&mut buffer, "hello world\r"),
        vec!["hello world".to_string()]
    );
    assert!(buffer.is_empty());
}

#[test]
fn assistant_input_treats_shift_enter_as_soft_newline() {
    let mut buffer = String::new();

    assert!(
        collect_submitted_assistant_prompts(&mut buffer, "line 1\u{001b}[13;2uline 2").is_empty()
    );
    assert_eq!(buffer, "line 1\nline 2");
    assert_eq!(
        collect_submitted_assistant_prompts(&mut buffer, "\r"),
        vec!["line 1\nline 2".to_string()]
    );
    assert!(buffer.is_empty());
}

#[test]
fn assistant_input_normalizes_shift_enter_escape() {
    assert_eq!(
        normalize_assistant_input("hello\u{001b}[13;2uworld"),
        format!("hello{SOFT_NEWLINE_SENTINEL}world")
    );
}

#[test]
fn turn_output_timeout_adds_startup_grace_only_before_first_output() {
    assert_eq!(
        turn_output_timeout(true, false),
        TURN_OUTPUT_IDLE_TIMEOUT + INITIAL_PROMPT_STARTUP_GRACE
    );
    assert_eq!(turn_output_timeout(true, true), TURN_OUTPUT_IDLE_TIMEOUT);
    assert_eq!(turn_output_timeout(false, false), TURN_OUTPUT_IDLE_TIMEOUT);
}

#[test]
fn sanitize_command_name_normalizes_silo_assistants() {
    assert_eq!(sanitize_command_name("silo codex \"ship it\""), "codex");
    assert_eq!(sanitize_command_name("silo claude \"ship it\""), "claude");
}

#[test]
fn zmx_session_parser_reads_name_and_command() {
    let session = parse_zmx_session("session_name=terminal-1\tpid=2\tcmd=codex")
        .expect("session should parse");
    assert_eq!(session.name, "terminal-1");
    assert_eq!(session.command.as_deref(), Some("codex"));
}

#[test]
fn assistant_provider_resolution_handles_cc_alias() {
    assert_eq!(
        resolve_assistant_provider("cc"),
        Some(AssistantProvider::Claude)
    );
    assert_eq!(
        resolve_assistant_provider("codex"),
        Some(AssistantProvider::Codex)
    );
    assert_eq!(resolve_assistant_provider("bun run dev"), None);
}

#[test]
fn flat_metadata_items_rewrites_terminal_state() {
    let mut items = BTreeMap::new();
    items.insert("target_branch".to_string(), "main".to_string());
    items.insert(
        "terminal-session-old".to_string(),
        "{\"type\":\"terminal\",\"name\":\"old\",\"attachment_id\":\"old\"}".to_string(),
    );
    let published = PublishedState {
        branch: Some("feature/inbox".to_string()),
        working: false,
        unread: true,
        heartbeat_at: "2026-03-14T00:00:00Z".to_string(),
        last_active: Some("2026-03-14T00:00:00Z".to_string()),
        last_working: Some("2026-03-14T01:00:00Z".to_string()),
        terminals: vec![PublishedSession {
            kind: "terminal",
            name: "codex".to_string(),
            attachment_id: "terminal-1".to_string(),
            url: None,
            title: None,
            favicon_url: None,
            can_go_back: None,
            can_go_forward: None,
            working: Some(false),
            unread: Some(true),
        }],
    };

    let items = flat_metadata_items(items, &published).expect("metadata items should build");

    assert_eq!(
        items.get("branch").map(String::as_str),
        Some("feature/inbox")
    );
    assert_eq!(
        items.get(TERMINAL_UNREAD_METADATA_KEY).map(String::as_str),
        Some("true")
    );
    assert_eq!(
        items.get(TERMINAL_WORKING_METADATA_KEY).map(String::as_str),
        Some("false")
    );
    assert_eq!(
        items
            .get(TERMINAL_LAST_ACTIVE_METADATA_KEY)
            .map(String::as_str),
        Some("2026-03-14T00:00:00Z")
    );
    assert_eq!(
        items
            .get(TERMINAL_LAST_WORKING_METADATA_KEY)
            .map(String::as_str),
        Some("2026-03-14T01:00:00Z")
    );
    assert_eq!(items.get("target_branch").map(String::as_str), Some("main"));
    assert_eq!(
        items
            .get("terminal-session-terminal-1")
            .map(String::as_str),
        Some(
            "{\"type\":\"terminal\",\"name\":\"codex\",\"attachment_id\":\"terminal-1\",\"working\":false,\"unread\":true}"
        )
    );
    assert!(!items.contains_key("terminal-session-old"));
}

#[test]
fn update_metadata_item_removes_empty_values() {
    let mut items = BTreeMap::new();
    items.insert(
        "last_active".to_string(),
        "2026-03-14T00:00:00Z".to_string(),
    );

    update_metadata_item(&mut items, "last_active", Some("   "));
    update_metadata_item(&mut items, "branch", Some("feature/inbox"));

    assert!(!items.contains_key("last_active"));
    assert_eq!(
        items.get("branch").map(String::as_str),
        Some("feature/inbox")
    );
    assert_eq!(bool_metadata_value(true), "true");
    assert_eq!(bool_metadata_value(false), "false");
}

#[test]
fn assistant_prompt_submitted_updates_last_working() {
    let mut state = ObserverState::default();

    apply_event(
        &mut state,
        ObserverEvent::AssistantPromptSubmitted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
        },
    );

    assert!(state.last_active.is_some());
    assert!(state.last_working.is_some());
    assert_eq!(
        state
            .sessions
            .get("terminal-1")
            .and_then(|session| session.working.then_some(true)),
        Some(true)
    );
}

#[test]
fn shell_session_lifecycle_events_manage_presence() {
    let mut state = ObserverState::default();

    apply_event(
        &mut state,
        ObserverEvent::ShellSessionStarted {
            session: "terminal-1".to_string(),
        },
    );
    assert!(state
        .sessions
        .get("terminal-1")
        .map(|session| session.lifecycle_managed)
        .unwrap_or(false));

    apply_event(
        &mut state,
        ObserverEvent::ShellSessionExited {
            session: "terminal-1".to_string(),
        },
    );
    assert!(!state.sessions.contains_key("terminal-1"));
}

#[test]
fn reconcile_sessions_keeps_lifecycle_managed_session_on_transient_poll_miss() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: Some("codex".to_string()),
            assistant_provider: Some(AssistantProvider::Codex),
            command_running: true,
            working: true,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
        },
    );

    reconcile_sessions(&mut state, &[]);

    assert!(state.sessions.contains_key("terminal-1"));
    assert_eq!(
        state
            .sessions
            .get("terminal-1")
            .map(|session| session.poll_misses),
        Some(1)
    );
}

#[test]
fn reconcile_sessions_clears_finished_assistant_back_to_shell() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: Some("claude".to_string()),
            assistant_provider: Some(AssistantProvider::Claude),
            command_running: false,
            working: false,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
        },
    );

    reconcile_sessions(
        &mut state,
        &[ZmxSession {
            name: "terminal-1".to_string(),
            command: None,
        }],
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should remain");
    assert_eq!(session.active_command, None);
    assert_eq!(session.assistant_provider, None);
}

#[test]
fn reconcile_sessions_drops_missing_idle_shell_session_quickly() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: None,
            assistant_provider: None,
            command_running: false,
            working: false,
            unread: true,
            lifecycle_managed: true,
            poll_misses: 0,
        },
    );

    for _ in 0..3 {
        reconcile_sessions(&mut state, &[]);
    }

    assert!(
        !state.sessions.contains_key("terminal-1"),
        "missing idle shell session should be evicted after the short threshold"
    );
}

#[test]
fn reconcile_sessions_preserves_running_command_without_live_cmd() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: Some("bun run dev".to_string()),
            assistant_provider: None,
            command_running: true,
            working: false,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
        },
    );

    reconcile_sessions(
        &mut state,
        &[ZmxSession {
            name: "terminal-1".to_string(),
            command: None,
        }],
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should remain");
    assert_eq!(session.active_command.as_deref(), Some("bun run dev"));
    assert_eq!(session.assistant_provider, None);
    assert!(session.command_running);
}

#[test]
fn assistant_session_started_marks_claude_active_immediately() {
    let mut state = ObserverState::default();

    apply_event(
        &mut state,
        ObserverEvent::AssistantSessionStarted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Claude,
        },
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert_eq!(session.active_command.as_deref(), Some("claude"));
    assert_eq!(session.assistant_provider, Some(AssistantProvider::Claude));
    assert!(session.command_running);
    assert!(!session.working);
    assert!(!session.unread);
}

#[test]
fn build_published_state_uses_agent_state_without_live_poll_data() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: Some("codex".to_string()),
            assistant_provider: Some(AssistantProvider::Codex),
            command_running: true,
            working: true,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 2,
        },
    );

    let published = build_published_state(&state);

    assert!(published.working);
    assert!(!published.unread);
    assert_eq!(published.terminals.len(), 1);
    assert_eq!(published.terminals[0].name, "codex");
    assert_eq!(published.terminals[0].attachment_id, "terminal-1");
    assert_eq!(published.terminals[0].working, Some(true));
}

#[test]
fn effective_activity_prefers_most_recent_timestamp() {
    let state = ObserverState {
        last_active: Some("2026-03-14T00:00:00Z".to_string()),
        last_working: Some("2026-03-14T03:00:00Z".to_string()),
        ..ObserverState::default()
    };

    assert_eq!(
        effective_activity_at(&state),
        parse_timestamp("2026-03-14T03:00:00Z")
    );
}

#[test]
fn effective_activity_falls_back_to_last_active() {
    let state = ObserverState {
        last_active: Some("2026-03-14T00:00:00Z".to_string()),
        ..ObserverState::default()
    };

    assert_eq!(
        effective_activity_at(&state),
        parse_timestamp("2026-03-14T00:00:00Z")
    );
}

#[test]
fn idle_suspend_requires_four_hours_of_inactivity() {
    let last_active = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");
    let now = last_active + TimeDuration::hours(4);
    let state = ObserverState {
        last_active: Some("2026-03-14T00:00:00Z".to_string()),
        last_working: Some("2026-03-13T23:00:00Z".to_string()),
        ..ObserverState::default()
    };

    assert!(should_suspend_for_inactivity_at(&state, now, false));
    assert!(!should_suspend_for_inactivity_at(
        &state,
        now - TimeDuration::seconds(1),
        false
    ));
}

#[test]
fn idle_suspend_does_not_fire_while_working() {
    let state = ObserverState {
        last_active: Some("2026-03-14T00:00:00Z".to_string()),
        last_working: Some("2026-03-14T00:00:00Z".to_string()),
        ..ObserverState::default()
    };
    let now = parse_timestamp("2026-03-14T05:00:00Z").expect("timestamp should parse");

    assert!(!should_suspend_for_inactivity_at(&state, now, true));
}

#[test]
fn lifecycle_poll_threshold_constant_is_preserved() {
    assert_eq!(POLL_MISS_THRESHOLD_LIFECYCLE, 300);
}
