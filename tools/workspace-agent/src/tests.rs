use std::collections::BTreeMap;

use time::Duration as TimeDuration;

use crate::assistant::observer_event_from_hook_input;
use crate::daemon::state::{
    apply_event, apply_event_at, build_published_state, effective_activity_at, parse_timestamp,
    reconcile_assistant_state_at, reconcile_sessions, resolve_assistant_provider,
    sanitize_command_name, should_suspend_for_inactivity_at, AssistantEvent, AssistantEventKind,
    AssistantProvider, ObserverEvent, ObserverState, PublishedActiveSession, PublishedSession,
    PublishedState, SessionState, POLL_MISS_THRESHOLD_LIFECYCLE,
};
use crate::daemon::zmx::{parse_zmx_session, parse_zmx_sessions, ZmxSession};
use crate::metadata::{
    bool_metadata_value, flat_metadata_items, update_metadata_item,
    TERMINAL_LAST_ACTIVE_METADATA_KEY, TERMINAL_LAST_WORKING_METADATA_KEY,
    TERMINAL_UNREAD_METADATA_KEY, TERMINAL_WORKING_METADATA_KEY,
};

#[test]
fn assistant_hook_maps_prompt_submit() {
    let event = observer_event_from_hook_input(
        "terminal-1",
        AssistantProvider::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"provider-session"}"#,
    );

    assert!(matches!(
        event,
        Some(ObserverEvent::AssistantEvent {
            session,
            provider: AssistantProvider::Claude,
            event:
                AssistantEvent {
                    kind: AssistantEventKind::UserPromptSubmit,
                    provider_session_id: Some(provider_session_id),
                    turn_id: None,
                    ..
                },
        }) if session == "terminal-1" && provider_session_id == "provider-session"
    ));
}

#[test]
fn assistant_hook_maps_stop() {
    let event = observer_event_from_hook_input(
        "terminal-1",
        AssistantProvider::Codex,
        r#"{"hookEventName":"Stop","session_id":"provider-session","turn_id":"turn-1"}"#,
    );

    assert!(matches!(
        event,
        Some(ObserverEvent::AssistantEvent {
            session,
            provider: AssistantProvider::Codex,
            event:
                AssistantEvent {
                    kind: AssistantEventKind::Stop,
                    provider_session_id: Some(provider_session_id),
                    turn_id: Some(turn_id),
                    ..
                },
        }) if session == "terminal-1"
            && provider_session_id == "provider-session"
            && turn_id == "turn-1"
    ));
}

#[test]
fn assistant_hook_maps_camel_case_prompt_submit_with_turn_id() {
    let event = observer_event_from_hook_input(
        "terminal-1",
        AssistantProvider::Codex,
        r#"{"hookEventName":"UserPromptSubmit","turn_id":"turn-1"}"#,
    );

    assert!(matches!(
        event,
        Some(ObserverEvent::AssistantEvent {
            session,
            provider: AssistantProvider::Codex,
            event:
                AssistantEvent {
                    kind: AssistantEventKind::UserPromptSubmit,
                    turn_id: Some(turn_id),
                    ..
                },
        }) if session == "terminal-1"
            && turn_id == "turn-1"
    ));
}

#[test]
fn assistant_hook_ignores_unknown_events() {
    let event = observer_event_from_hook_input(
        "terminal-1",
        AssistantProvider::Codex,
        r#"{"hook_event_name":"DefinitelyUnknown"}"#,
    );

    assert!(event.is_none());
}

#[test]
fn sanitize_command_name_normalizes_silo_assistants() {
    assert_eq!(sanitize_command_name("silo codex \"ship it\""), "codex");
    assert_eq!(sanitize_command_name("silo claude \"ship it\""), "claude");
}

#[test]
fn sanitize_command_name_normalizes_assistant_proxy_commands() {
    assert_eq!(
        sanitize_command_name(
            "/home/silo/.silo/bin/workspace-agent assistant-proxy --provider codex -- codex"
        ),
        "codex"
    );
    assert_eq!(
        sanitize_command_name(
            "'/home/silo/.silo/bin/workspace-agent' assistant-proxy --provider 'claude' -- 'claude'"
        ),
        "claude"
    );
}

#[test]
fn zmx_session_parser_reads_legacy_session_name_and_command() {
    let session = parse_zmx_session("session_name=terminal-1\tpid=2\tcmd=codex")
        .expect("session should parse");
    assert_eq!(session.name, "terminal-1");
    assert_eq!(session.command.as_deref(), Some("codex"));
}

#[test]
fn zmx_session_parser_reads_modern_name_field() {
    let session =
        parse_zmx_session("name=terminal-1\tpid=2\tclients=0").expect("session should parse");
    assert_eq!(session.name, "terminal-1");
    assert_eq!(session.command, None);
}

#[test]
fn zmx_sessions_parser_accepts_no_sessions_output() {
    let sessions =
        parse_zmx_sessions("no sessions found in /run/user/1001/zmx").expect("output should parse");
    assert!(sessions.is_empty());
}

#[test]
fn zmx_sessions_parser_rejects_unknown_output() {
    let error =
        parse_zmx_sessions("name terminal-1").expect_err("invalid output should be rejected");
    assert!(error.contains("failed to parse zmx session line"));
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
    assert_eq!(
        resolve_assistant_provider(
            "/home/silo/.silo/bin/workspace-agent assistant-proxy --provider codex -- codex"
        ),
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
            kind: "terminal".to_string(),
            name: "codex".to_string(),
            attachment_id: "terminal-1".to_string(),
            path: None,
            url: None,
            logical_url: None,
            resolved_url: None,
            title: None,
            favicon_url: None,
            can_go_back: None,
            can_go_forward: None,
            working: Some(false),
            unread: Some(true),
        }],
        active_session: None,
        browsers: Vec::new(),
        files: Vec::new(),
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
        items.get("terminal-session-old").map(String::as_str),
        Some("{\"type\":\"terminal\",\"name\":\"old\",\"attachment_id\":\"old\"}")
    );
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
            turn_id: Some("turn-1".to_string()),
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
fn session_upsert_persists_browser_and_file_sessions() {
    let mut state = ObserverState::default();

    apply_event(
        &mut state,
        ObserverEvent::SessionUpsert {
            session: PublishedSession {
                kind: "browser".to_string(),
                name: "Docs".to_string(),
                attachment_id: "browser-1".to_string(),
                path: None,
                url: Some("https://example.com".to_string()),
                logical_url: Some("https://example.com".to_string()),
                resolved_url: Some("https://example.com".to_string()),
                title: Some("Docs".to_string()),
                favicon_url: None,
                can_go_back: Some(false),
                can_go_forward: Some(false),
                working: Some(false),
                unread: Some(false),
            },
        },
    );
    apply_event(
        &mut state,
        ObserverEvent::SessionUpsert {
            session: PublishedSession {
                kind: "file".to_string(),
                name: "README.md".to_string(),
                attachment_id: "file-1".to_string(),
                path: Some("README.md".to_string()),
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: None,
                unread: None,
            },
        },
    );

    let published = build_published_state(&state);
    assert_eq!(published.browsers.len(), 1);
    assert_eq!(published.files.len(), 1);
    assert_eq!(published.browsers[0].attachment_id, "browser-1");
    assert_eq!(published.files[0].attachment_id, "file-1");
}

#[test]
fn active_session_is_cleared_when_target_session_disappears() {
    let mut state = ObserverState {
        active_session: Some(PublishedActiveSession {
            kind: "browser".to_string(),
            attachment_id: "browser-1".to_string(),
        }),
        ..ObserverState::default()
    };
    state.browsers.insert(
        "browser-1".to_string(),
        PublishedSession {
            kind: "browser".to_string(),
            name: "Docs".to_string(),
            attachment_id: "browser-1".to_string(),
            path: None,
            url: Some("https://example.com".to_string()),
            logical_url: Some("https://example.com".to_string()),
            resolved_url: Some("https://example.com".to_string()),
            title: Some("Docs".to_string()),
            favicon_url: None,
            can_go_back: Some(false),
            can_go_forward: Some(false),
            working: Some(false),
            unread: Some(false),
        },
    );

    apply_event(
        &mut state,
        ObserverEvent::SessionRemove {
            session_type: "browser".to_string(),
            attachment_id: "browser-1".to_string(),
        },
    );

    assert_eq!(state.active_session, None);
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
            active_turn_id: None,
            completed_turn_id: None,
            command_running: true,
            working: true,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
            ..SessionState::default()
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
            active_turn_id: None,
            completed_turn_id: None,
            command_running: false,
            working: false,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
            ..SessionState::default()
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
            active_turn_id: None,
            completed_turn_id: None,
            command_running: false,
            working: false,
            unread: true,
            lifecycle_managed: true,
            poll_misses: 0,
            ..SessionState::default()
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
            active_turn_id: None,
            completed_turn_id: None,
            command_running: true,
            working: false,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 0,
            ..SessionState::default()
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
            active_turn_id: Some("turn-1".to_string()),
            completed_turn_id: None,
            command_running: true,
            working: true,
            unread: false,
            lifecycle_managed: true,
            poll_misses: 2,
            ..SessionState::default()
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
fn build_published_state_canonicalizes_assistant_proxy_session_names() {
    let mut state = ObserverState::default();
    state.sessions.insert(
        "terminal-1".to_string(),
        SessionState {
            active_command: Some(
                "/home/silo/.silo/bin/workspace-agent assistant-proxy --provider codex -- codex"
                    .to_string(),
            ),
            assistant_provider: None,
            active_turn_id: None,
            completed_turn_id: None,
            command_running: true,
            working: false,
            unread: true,
            lifecycle_managed: true,
            poll_misses: 0,
            ..SessionState::default()
        },
    );

    let published = build_published_state(&state);

    assert_eq!(published.terminals.len(), 1);
    assert_eq!(published.terminals[0].name, "codex");
    assert_eq!(published.terminals[0].working, Some(false));
    assert_eq!(published.terminals[0].unread, Some(true));
}

#[test]
fn assistant_turn_completed_ignores_stale_turn_ids() {
    let mut state = ObserverState::default();

    apply_event(
        &mut state,
        ObserverEvent::AssistantPromptSubmitted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-2".to_string()),
        },
    );
    apply_event(
        &mut state,
        ObserverEvent::AssistantTurnCompleted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(session.working);
    assert!(!session.unread);
    assert_eq!(session.active_turn_id.as_deref(), Some("turn-2"));
}

#[test]
fn assistant_turn_completed_ignores_duplicate_turn_ids_after_mark_read() {
    let mut state = ObserverState::default();
    let start = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantPromptSubmitted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
        start,
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantTurnCompleted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
        start + TimeDuration::seconds(1),
    );
    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(7));
    apply_event_at(
        &mut state,
        ObserverEvent::MarkRead {
            session: "terminal-1".to_string(),
        },
        start + TimeDuration::seconds(8),
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantTurnCompleted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
        start + TimeDuration::seconds(9),
    );
    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(15));

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(!session.working);
    assert!(!session.unread);
    assert_eq!(session.completed_turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn codex_completion_waits_for_settle_window_before_unread() {
    let mut state = ObserverState::default();
    let start = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantPromptSubmitted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
        start,
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantTurnCompleted {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            turn_id: Some("turn-1".to_string()),
        },
        start + TimeDuration::seconds(1),
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(session.working);
    assert!(!session.unread);

    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(5));
    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(session.working);
    assert!(!session.unread);

    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(6));
    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(!session.working);
    assert!(session.unread);
}

#[test]
fn codex_nested_activity_defers_completion_until_quiet() {
    let mut state = ObserverState::default();
    let start = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::UserPromptSubmit,
                provider_session_id: Some("root-session".to_string()),
                turn_id: Some("turn-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start,
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::Stop,
                provider_session_id: Some("root-session".to_string()),
                turn_id: Some("turn-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(1),
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::PreToolUse,
                provider_session_id: Some("nested-session".to_string()),
                tool_name: Some("Bash".to_string()),
                tool_call_id: Some("tool-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(2),
    );

    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(8));
    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(session.working);
    assert!(!session.unread);

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::PostToolUse,
                provider_session_id: Some("nested-session".to_string()),
                tool_name: Some("Bash".to_string()),
                tool_call_id: Some("tool-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(9),
    );
    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(9));

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(!session.working);
    assert!(session.unread);
}

#[test]
fn codex_leaked_root_tools_clear_after_settle_window() {
    let mut state = ObserverState::default();
    let start = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::UserPromptSubmit,
                provider_session_id: Some("root-session".to_string()),
                turn_id: Some("turn-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start,
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::PreToolUse,
                provider_session_id: Some("root-session".to_string()),
                tool_name: Some("Bash".to_string()),
                tool_call_id: Some("tool-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(1),
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Codex,
            event: AssistantEvent {
                kind: AssistantEventKind::Stop,
                provider_session_id: Some("root-session".to_string()),
                turn_id: Some("turn-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(2),
    );

    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(6));
    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(session.working);
    assert!(!session.unread);
    assert_eq!(session.active_tools.len(), 1);

    reconcile_assistant_state_at(&mut state, start + TimeDuration::seconds(7));
    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(!session.working);
    assert!(session.unread);
    assert!(session.active_tools.is_empty());
    assert_eq!(session.completed_turn_id.as_deref(), Some("turn-1"));
}

#[test]
fn claude_attention_events_set_unread_without_working() {
    let mut state = ObserverState::default();
    let start = parse_timestamp("2026-03-14T00:00:00Z").expect("timestamp should parse");

    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Claude,
            event: AssistantEvent {
                kind: AssistantEventKind::UserPromptSubmit,
                provider_session_id: Some("claude-session".to_string()),
                turn_id: Some("turn-1".to_string()),
                ..AssistantEvent::default()
            },
        },
        start,
    );
    apply_event_at(
        &mut state,
        ObserverEvent::AssistantEvent {
            session: "terminal-1".to_string(),
            provider: AssistantProvider::Claude,
            event: AssistantEvent {
                kind: AssistantEventKind::PermissionRequest,
                provider_session_id: Some("claude-session".to_string()),
                ..AssistantEvent::default()
            },
        },
        start + TimeDuration::seconds(1),
    );

    let session = state
        .sessions
        .get("terminal-1")
        .expect("session should exist");
    assert!(!session.working);
    assert!(session.unread);
    assert!(session.attention_pending);
    assert_eq!(
        session.blocked_reason.as_deref(),
        Some("permission_request")
    );
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
