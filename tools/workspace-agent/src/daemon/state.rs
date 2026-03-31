use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};

use super::zmx::ZmxSession;

const HEARTBEAT_PUBLISH_INTERVAL: TimeDuration = TimeDuration::seconds(15);
const AUTO_SUSPEND_IDLE_THRESHOLD: TimeDuration = TimeDuration::hours(4);
const POLL_MISS_THRESHOLD_UNMANAGED: u16 = 3;
pub(crate) const POLL_MISS_THRESHOLD_LIFECYCLE: u16 = 300;
const CODEX_COMPLETION_SETTLE_WINDOW: TimeDuration = TimeDuration::seconds(5);
const CLAUDE_COMPLETION_SETTLE_WINDOW: TimeDuration = TimeDuration::seconds(2);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct ObserverState {
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) last_active: Option<String>,
    #[serde(default)]
    pub(crate) last_working: Option<String>,
    #[serde(default)]
    pub(crate) active_session: Option<PublishedActiveSession>,
    #[serde(default)]
    pub(crate) sessions: BTreeMap<String, SessionState>,
    #[serde(default)]
    pub(crate) browsers: BTreeMap<String, PublishedSession>,
    #[serde(default)]
    pub(crate) files_sessions: BTreeMap<String, PublishedSession>,
    #[serde(default)]
    pub(crate) files: FilesState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct FilesState {
    #[serde(default)]
    pub(crate) watch_paths: BTreeSet<String>,
    #[serde(default)]
    pub(crate) watched: BTreeMap<String, FileWatchState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct FileWatchState {
    pub(crate) exists: bool,
    pub(crate) binary: bool,
    pub(crate) revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct SessionState {
    #[serde(default)]
    pub(crate) active_command: Option<String>,
    #[serde(default)]
    pub(crate) assistant_provider: Option<AssistantProvider>,
    #[serde(default)]
    pub(crate) root_provider_session_id: Option<String>,
    #[serde(default)]
    pub(crate) active_turn_id: Option<String>,
    #[serde(default)]
    pub(crate) completed_turn_id: Option<String>,
    #[serde(default)]
    pub(crate) pending_completion: Option<PendingCompletionState>,
    #[serde(default)]
    pub(crate) root_turn_active: bool,
    #[serde(default)]
    pub(crate) active_subagents: BTreeSet<String>,
    #[serde(default)]
    pub(crate) active_tasks: BTreeSet<String>,
    #[serde(default)]
    pub(crate) active_tools: BTreeSet<String>,
    #[serde(default)]
    pub(crate) compacting: bool,
    #[serde(default)]
    pub(crate) attention_pending: bool,
    #[serde(default)]
    pub(crate) blocked_reason: Option<String>,
    #[serde(default)]
    pub(crate) command_running: bool,
    #[serde(default)]
    pub(crate) working: bool,
    #[serde(default)]
    pub(crate) unread: bool,
    #[serde(default)]
    pub(crate) lifecycle_managed: bool,
    #[serde(default)]
    pub(crate) poll_misses: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PendingCompletionState {
    #[serde(default)]
    pub(crate) provider_session_id: Option<String>,
    #[serde(default)]
    pub(crate) turn_id: Option<String>,
    pub(crate) settle_until: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssistantEventKind {
    #[default]
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PermissionRequest,
    Notification,
    SubagentStart,
    SubagentStop,
    TaskCreated,
    TaskCompleted,
    Stop,
    StopFailure,
    TeammateIdle,
    PreCompact,
    PostCompact,
    SessionEnd,
    Elicitation,
    ElicitationResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct AssistantEvent {
    pub(crate) kind: AssistantEventKind,
    #[serde(default)]
    pub(crate) provider_session_id: Option<String>,
    #[serde(default)]
    pub(crate) turn_id: Option<String>,
    #[serde(default)]
    pub(crate) tool_name: Option<String>,
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default)]
    pub(crate) notification_type: Option<String>,
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) agent_type: Option<String>,
    #[serde(default)]
    pub(crate) task_id: Option<String>,
    #[serde(default)]
    pub(crate) compact_trigger: Option<String>,
}

fn reset_assistant_session_state(session_state: &mut SessionState) {
    session_state.root_provider_session_id = None;
    session_state.active_turn_id = None;
    session_state.completed_turn_id = None;
    session_state.pending_completion = None;
    session_state.root_turn_active = false;
    session_state.active_subagents.clear();
    session_state.active_tasks.clear();
    session_state.active_tools.clear();
    session_state.compacting = false;
    session_state.attention_pending = false;
    session_state.blocked_reason = None;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PublishedState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) branch: Option<String>,
    pub(crate) working: bool,
    pub(crate) unread: bool,
    pub(crate) heartbeat_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_working: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) active_session: Option<PublishedActiveSession>,
    pub(crate) terminals: Vec<PublishedSession>,
    #[serde(default)]
    pub(crate) browsers: Vec<PublishedSession>,
    #[serde(default)]
    pub(crate) files: Vec<PublishedSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PublishedSession {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) attachment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) logical_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) favicon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) can_go_back: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) can_go_forward: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) working: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unread: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PublishedActiveSession {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) attachment_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssistantProvider {
    Codex,
    Claude,
}

impl AssistantProvider {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }

    pub(crate) fn command_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum ObserverEvent {
    ShellSessionStarted {
        session: String,
    },
    ShellSessionExited {
        session: String,
    },
    ShellCommandStarted {
        session: String,
        command: String,
    },
    ShellCommandFinished {
        session: String,
    },
    AssistantSessionStarted {
        session: String,
        provider: AssistantProvider,
    },
    AssistantPromptSubmitted {
        session: String,
        provider: AssistantProvider,
        turn_id: Option<String>,
    },
    AssistantTurnCompleted {
        session: String,
        provider: AssistantProvider,
        turn_id: Option<String>,
    },
    AssistantEvent {
        session: String,
        provider: AssistantProvider,
        event: AssistantEvent,
    },
    MarkRead {
        session: String,
    },
    SessionUpsert {
        session: PublishedSession,
    },
    SessionRemove {
        session_type: String,
        attachment_id: String,
    },
    SetActiveSession {
        session_type: String,
        attachment_id: String,
    },
    ClearActiveSession,
    FilesWatchSet {
        paths: Vec<String>,
    },
}

pub(crate) fn apply_event(state: &mut ObserverState, event: ObserverEvent) {
    apply_event_at(state, event, OffsetDateTime::now_utc());
}

pub(crate) fn apply_event_at(state: &mut ObserverState, event: ObserverEvent, now: OffsetDateTime) {
    touch_last_active_at(state, now);
    match event {
        ObserverEvent::ShellSessionStarted { session } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.lifecycle_managed = true;
            session_state.poll_misses = 0;
        }
        ObserverEvent::ShellSessionExited { session } => {
            state.sessions.remove(&session);
        }
        ObserverEvent::ShellCommandStarted { session, command } => {
            let command = sanitize_command_name(&command);
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(command.clone());
            session_state.assistant_provider = resolve_assistant_provider(&command);
            reset_assistant_session_state(session_state);
            session_state.command_running = true;
            session_state.working = false;
            session_state.unread = false;
            session_state.poll_misses = 0;
        }
        ObserverEvent::ShellCommandFinished { session } => {
            if let Some(session_state) = state.sessions.get_mut(&session) {
                session_state.active_command = None;
                session_state.assistant_provider = None;
                reset_assistant_session_state(session_state);
                session_state.command_running = false;
                session_state.working = false;
                session_state.unread = false;
                session_state.poll_misses = 0;
            }
        }
        ObserverEvent::AssistantSessionStarted { session, provider } => {
            apply_assistant_event_at(
                state,
                session,
                provider,
                AssistantEvent {
                    kind: AssistantEventKind::SessionStart,
                    ..AssistantEvent::default()
                },
                now,
            );
        }
        ObserverEvent::AssistantPromptSubmitted {
            session,
            provider,
            turn_id,
        } => {
            apply_assistant_event_at(
                state,
                session,
                provider,
                AssistantEvent {
                    kind: AssistantEventKind::UserPromptSubmit,
                    turn_id,
                    ..AssistantEvent::default()
                },
                now,
            );
        }
        ObserverEvent::AssistantTurnCompleted {
            session,
            provider,
            turn_id,
        } => {
            apply_assistant_event_at(
                state,
                session,
                provider,
                AssistantEvent {
                    kind: AssistantEventKind::Stop,
                    turn_id,
                    ..AssistantEvent::default()
                },
                now,
            );
        }
        ObserverEvent::AssistantEvent {
            session,
            provider,
            event,
        } => {
            apply_assistant_event_at(state, session, provider, event, now);
        }
        ObserverEvent::MarkRead { session } => {
            if let Some(session_state) = state.sessions.get_mut(&session) {
                session_state.unread = false;
            }
        }
        ObserverEvent::SessionUpsert { session } => {
            upsert_persisted_session(state, session);
        }
        ObserverEvent::SessionRemove {
            session_type,
            attachment_id,
        } => {
            remove_persisted_session(state, &session_type, &attachment_id);
        }
        ObserverEvent::SetActiveSession {
            session_type,
            attachment_id,
        } => {
            state.active_session = Some(PublishedActiveSession {
                kind: session_type,
                attachment_id,
            });
        }
        ObserverEvent::ClearActiveSession => {
            state.active_session = None;
        }
        ObserverEvent::FilesWatchSet { paths } => {
            state.files.watch_paths = paths.into_iter().collect();
            state
                .files
                .watched
                .retain(|path, _| state.files.watch_paths.contains(path));
        }
    }
    reconcile_assistant_state_at(state, now);
    reconcile_active_session(state);
}

pub(crate) fn reconcile_sessions(state: &mut ObserverState, live_sessions: &[ZmxSession]) {
    let live = live_sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect::<Vec<_>>();
    let before = state.sessions.clone();

    for live_session in live_sessions {
        let session_state = state.sessions.entry(live_session.name.clone()).or_default();
        session_state.poll_misses = 0;
        let live_command = live_session.command.as_deref().map(sanitize_command_name);
        if let Some(command) = live_command {
            session_state.active_command = Some(command.clone());
            session_state.assistant_provider = resolve_assistant_provider(&command);
            session_state.command_running = true;
        } else if !session_state.command_running && !session_state.working && !session_state.unread
        {
            session_state.active_command = None;
            session_state.assistant_provider = None;
            reset_assistant_session_state(session_state);
        }
    }

    for (name, session_state) in &mut state.sessions {
        if live.iter().any(|candidate| *candidate == name) {
            continue;
        }
        session_state.poll_misses = session_state.poll_misses.saturating_add(1);
    }

    state.sessions.retain(|_, session_state| {
        session_state.poll_misses < session_poll_miss_threshold(session_state)
    });
    reconcile_assistant_state_at(state, OffsetDateTime::now_utc());
    reconcile_active_session(state);

    if state.sessions != before {
        touch_last_active_at(state, OffsetDateTime::now_utc());
    }
}

pub(crate) fn build_published_state(state: &ObserverState) -> PublishedState {
    let mut working = false;
    let mut unread = false;
    let mut terminals = state
        .sessions
        .iter()
        .map(|(session_name, session_state)| {
            let assistant_provider = session_state.assistant_provider.or_else(|| {
                session_state
                    .active_command
                    .as_deref()
                    .and_then(resolve_assistant_provider)
            });
            let name = assistant_provider
                .map(|provider| provider.command_name().to_string())
                .or_else(|| session_state.active_command.clone())
                .unwrap_or_else(|| "shell".to_string());
            let assistant_capable = assistant_provider.is_some();
            working |= session_state.working;
            unread |= session_state.unread;

            PublishedSession {
                kind: "terminal".to_string(),
                name,
                attachment_id: session_name.clone(),
                path: None,
                url: None,
                logical_url: None,
                resolved_url: None,
                title: None,
                favicon_url: None,
                can_go_back: None,
                can_go_forward: None,
                working: assistant_capable.then_some(session_state.working),
                unread: assistant_capable.then_some(session_state.unread),
            }
        })
        .collect::<Vec<_>>();
    terminals.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));
    let mut browsers = state.browsers.values().cloned().collect::<Vec<_>>();
    browsers.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));
    let mut files = state.files_sessions.values().cloned().collect::<Vec<_>>();
    files.sort_by(|left, right| left.attachment_id.cmp(&right.attachment_id));

    PublishedState {
        branch: state.branch.clone(),
        working,
        unread,
        heartbeat_at: heartbeat_timestamp(),
        last_active: state.last_active.clone(),
        last_working: state.last_working.clone(),
        active_session: state
            .active_session
            .clone()
            .filter(|active| session_exists(state, &active.kind, &active.attachment_id)),
        terminals,
        browsers,
        files,
    }
}

fn heartbeat_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    let seconds = now.unix_timestamp();
    let bucket = HEARTBEAT_PUBLISH_INTERVAL.whole_seconds().max(1);
    let rounded = seconds - seconds.rem_euclid(bucket);
    OffsetDateTime::from_unix_timestamp(rounded)
        .unwrap_or(now)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn touch_last_active_at(state: &mut ObserverState, now: OffsetDateTime) {
    state.last_active = format_timestamp(now);
}

fn touch_last_working_at(state: &mut ObserverState, now: OffsetDateTime) {
    state.last_working = format_timestamp(now);
}

fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub(crate) fn effective_activity_at(state: &ObserverState) -> Option<OffsetDateTime> {
    [state.last_active.as_deref(), state.last_working.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(parse_timestamp)
        .max()
}

pub(crate) fn effective_activity_marker(state: &ObserverState) -> Option<String> {
    effective_activity_at(state).and_then(format_timestamp)
}

pub(crate) fn should_suspend_for_inactivity_at(
    state: &ObserverState,
    now: OffsetDateTime,
    working: bool,
) -> bool {
    if working {
        return false;
    }

    let Some(effective_activity_at) = effective_activity_at(state) else {
        return false;
    };

    now - effective_activity_at >= AUTO_SUSPEND_IDLE_THRESHOLD
}

pub(crate) fn parse_timestamp(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

fn format_timestamp(timestamp: OffsetDateTime) -> Option<String> {
    timestamp.format(&Rfc3339).ok()
}

fn session_poll_miss_threshold(session: &SessionState) -> u16 {
    if session.lifecycle_managed && (session.working || session.active_command.is_some()) {
        POLL_MISS_THRESHOLD_LIFECYCLE
    } else {
        POLL_MISS_THRESHOLD_UNMANAGED
    }
}

fn upsert_persisted_session(state: &mut ObserverState, session: PublishedSession) {
    let entry = match session.kind.as_str() {
        "browser" => &mut state.browsers,
        "file" => &mut state.files_sessions,
        _ => return,
    };
    entry.insert(session.attachment_id.clone(), session);
}

fn remove_persisted_session(state: &mut ObserverState, kind: &str, attachment_id: &str) {
    match kind {
        "browser" => {
            state.browsers.remove(attachment_id);
        }
        "file" => {
            state.files_sessions.remove(attachment_id);
        }
        "terminal" => {
            state.sessions.remove(attachment_id);
        }
        _ => {}
    }
    if state
        .active_session
        .as_ref()
        .is_some_and(|active| active.kind == kind && active.attachment_id == attachment_id)
    {
        state.active_session = None;
    }
}

fn session_exists(state: &ObserverState, kind: &str, attachment_id: &str) -> bool {
    match kind {
        "terminal" => state.sessions.contains_key(attachment_id),
        "browser" => state.browsers.contains_key(attachment_id),
        "file" => state.files_sessions.contains_key(attachment_id),
        _ => false,
    }
}

fn reconcile_active_session(state: &mut ObserverState) {
    if !state
        .active_session
        .as_ref()
        .is_some_and(|active| session_exists(state, &active.kind, &active.attachment_id))
    {
        state.active_session = None;
    }
}

pub(crate) fn reconcile_assistant_state_at(state: &mut ObserverState, now: OffsetDateTime) {
    for session_state in state.sessions.values_mut() {
        reconcile_session_completion(session_state, now);
        session_state.working = !session_state.attention_pending
            && (session_state.root_turn_active
                || !session_state.active_subagents.is_empty()
                || !session_state.active_tasks.is_empty()
                || !session_state.active_tools.is_empty()
                || session_state.compacting
                || session_state.pending_completion.is_some());
    }
}

fn reconcile_session_completion(session_state: &mut SessionState, now: OffsetDateTime) {
    let deadline_reached = match session_state
        .pending_completion
        .as_ref()
        .and_then(|pending| parse_timestamp(&pending.settle_until))
    {
        Some(deadline) => now >= deadline,
        None => true,
    };
    if deadline_reached {
        clear_stale_codex_root_tools_after_completion(session_state);
    }
    if !deadline_reached
        || session_state.root_turn_active
        || session_state.compacting
        || !session_state.active_subagents.is_empty()
        || !session_state.active_tasks.is_empty()
        || !session_state.active_tools.is_empty()
    {
        return;
    }

    let Some(pending) = session_state.pending_completion.take() else {
        return;
    };
    if let Some(turn_id) = pending.turn_id {
        session_state.completed_turn_id = Some(turn_id);
    }
    if !session_state.attention_pending
        && !session_state.root_turn_active
        && !session_state.compacting
        && session_state.active_subagents.is_empty()
        && session_state.active_tasks.is_empty()
        && session_state.active_tools.is_empty()
    {
        session_state.unread = true;
    }
}

fn clear_stale_codex_root_tools_after_completion(session_state: &mut SessionState) {
    if session_state.assistant_provider != Some(AssistantProvider::Codex)
        || session_state.pending_completion.is_none()
        || session_state.root_turn_active
        || session_state.compacting
        || !session_state.active_subagents.is_empty()
        || !session_state.active_tasks.is_empty()
        || session_state.active_tools.is_empty()
    {
        return;
    }

    let owner = session_state
        .pending_completion
        .as_ref()
        .and_then(|pending| pending.provider_session_id.as_deref())
        .or(session_state.root_provider_session_id.as_deref())
        .unwrap_or("root")
        .to_string();
    let owner_prefix = format!("{owner}:");
    let leaked_tools: Vec<String> = session_state
        .active_tools
        .iter()
        .filter(|tool| tool.starts_with(&owner_prefix))
        .cloned()
        .collect();
    if leaked_tools.is_empty() {
        return;
    }

    for tool in &leaked_tools {
        session_state.active_tools.remove(tool);
    }

    eprintln!(
        "workspace-agent: cleared {} leaked codex root tools after settled completion owner={} tools={}",
        leaked_tools.len(),
        owner,
        leaked_tools.join(",")
    );
}

fn apply_assistant_event_at(
    state: &mut ObserverState,
    session: String,
    provider: AssistantProvider,
    event: AssistantEvent,
    now: OffsetDateTime,
) {
    let mut touched_working = false;
    let session_state = state.sessions.entry(session).or_default();
    session_state.active_command = Some(provider.command_name().to_string());
    session_state.assistant_provider = Some(provider);
    session_state.command_running = true;
    session_state.poll_misses = 0;

    let scope = classify_assistant_scope(session_state, provider, &event);
    match event.kind {
        AssistantEventKind::SessionStart => match scope {
            AssistantScope::Root => {
                clear_pending_completion(session_state);
                clear_attention(session_state);
            }
            AssistantScope::Nested(owner) => {
                session_state.active_subagents.insert(owner);
                clear_attention(session_state);
                touched_working = true;
            }
        },
        AssistantEventKind::UserPromptSubmit => match scope {
            AssistantScope::Root => {
                clear_attention(session_state);
                clear_pending_completion(session_state);
                session_state.root_turn_active = true;
                session_state.active_turn_id = event.turn_id;
                session_state.completed_turn_id = None;
                session_state.unread = false;
                touched_working = true;
            }
            AssistantScope::Nested(owner) => {
                session_state.active_subagents.insert(owner);
                clear_attention(session_state);
                touched_working = true;
            }
        },
        AssistantEventKind::PreToolUse => {
            clear_attention(session_state);
            if matches!(scope, AssistantScope::Root) {
                clear_pending_completion(session_state);
            }
            session_state
                .active_tools
                .insert(tool_activity_key(&scope, &event));
            touched_working = true;
        }
        AssistantEventKind::PostToolUse | AssistantEventKind::PostToolUseFailure => {
            session_state
                .active_tools
                .remove(&tool_activity_key(&scope, &event));
        }
        AssistantEventKind::PermissionRequest | AssistantEventKind::Elicitation => {
            clear_pending_completion(session_state);
            session_state.attention_pending = true;
            session_state.blocked_reason = Some(match event.kind {
                AssistantEventKind::PermissionRequest => "permission_request".to_string(),
                _ => "elicitation".to_string(),
            });
            session_state.unread = true;
        }
        AssistantEventKind::Notification => {
            if matches!(
                event.notification_type.as_deref(),
                Some("permission_prompt" | "idle_prompt" | "elicitation_dialog")
            ) {
                clear_pending_completion(session_state);
                session_state.attention_pending = true;
                session_state.blocked_reason = event.notification_type;
                session_state.unread = true;
            }
        }
        AssistantEventKind::ElicitationResult => {
            clear_pending_completion(session_state);
            clear_attention(session_state);
            touched_working = true;
        }
        AssistantEventKind::SubagentStart => {
            if let Some(owner) = nested_owner(&scope, &event) {
                session_state.active_subagents.insert(owner);
                clear_attention(session_state);
                touched_working = true;
            }
        }
        AssistantEventKind::SubagentStop => {
            if let Some(owner) = nested_owner(&scope, &event) {
                clear_activity_owner(session_state, &owner);
            }
        }
        AssistantEventKind::TaskCreated => {
            clear_attention(session_state);
            if matches!(scope, AssistantScope::Root) {
                clear_pending_completion(session_state);
            }
            if let Some(task_key) = task_activity_key(&scope, &event) {
                session_state.active_tasks.insert(task_key);
                touched_working = true;
            }
        }
        AssistantEventKind::TaskCompleted => {
            if let Some(task_key) = task_activity_key(&scope, &event) {
                session_state.active_tasks.remove(&task_key);
            }
        }
        AssistantEventKind::Stop => match scope {
            AssistantScope::Root => {
                if should_ignore_root_completion(session_state, &event) {
                    return;
                }
                clear_attention(session_state);
                session_state.root_turn_active = false;
                session_state.active_turn_id = None;
                session_state.pending_completion = Some(PendingCompletionState {
                    provider_session_id: event
                        .provider_session_id
                        .clone()
                        .or_else(|| session_state.root_provider_session_id.clone()),
                    turn_id: event.turn_id,
                    settle_until: format_timestamp(now + completion_settle_window(provider))
                        .unwrap_or_else(current_timestamp),
                });
                session_state.unread = false;
            }
            AssistantScope::Nested(owner) => {
                clear_activity_owner(session_state, &owner);
            }
        },
        AssistantEventKind::StopFailure => {
            clear_pending_completion(session_state);
            session_state.root_turn_active = false;
            session_state.active_turn_id = None;
            session_state.attention_pending = true;
            session_state.blocked_reason = Some("stop_failure".to_string());
            session_state.unread = true;
        }
        AssistantEventKind::PreCompact => {
            clear_attention(session_state);
            session_state.compacting = true;
            touched_working = true;
        }
        AssistantEventKind::PostCompact => {
            session_state.compacting = false;
            touched_working = true;
        }
        AssistantEventKind::SessionEnd => {
            clear_pending_completion(session_state);
            clear_attention(session_state);
            session_state.root_turn_active = false;
            session_state.active_turn_id = None;
            session_state.active_subagents.clear();
            session_state.active_tasks.clear();
            session_state.active_tools.clear();
            session_state.compacting = false;
            if let Some(provider_session_id) = event.provider_session_id {
                if session_state.root_provider_session_id.as_deref()
                    == Some(provider_session_id.as_str())
                {
                    session_state.root_provider_session_id = None;
                }
            }
        }
        AssistantEventKind::TeammateIdle => {}
    }
    if touched_working {
        touch_last_working_at(state, now);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssistantScope {
    Root,
    Nested(String),
}

fn classify_assistant_scope(
    session_state: &mut SessionState,
    provider: AssistantProvider,
    event: &AssistantEvent,
) -> AssistantScope {
    if let Some(agent_id) = event.agent_id.clone() {
        return AssistantScope::Nested(agent_id);
    }

    let provider_session_id = event.provider_session_id.clone();
    if let Some(provider_session_id) = provider_session_id {
        match session_state.root_provider_session_id.clone() {
            Some(root) if root == provider_session_id => AssistantScope::Root,
            Some(_) if provider == AssistantProvider::Codex => {
                AssistantScope::Nested(provider_session_id)
            }
            Some(_) => AssistantScope::Root,
            None => {
                session_state.root_provider_session_id = Some(provider_session_id);
                AssistantScope::Root
            }
        }
    } else {
        AssistantScope::Root
    }
}

fn nested_owner(scope: &AssistantScope, event: &AssistantEvent) -> Option<String> {
    match scope {
        AssistantScope::Nested(owner) => Some(owner.clone()),
        AssistantScope::Root => event.agent_id.clone(),
    }
}

fn tool_activity_key(scope: &AssistantScope, event: &AssistantEvent) -> String {
    let owner = match scope {
        AssistantScope::Root => event.provider_session_id.as_deref().unwrap_or("root"),
        AssistantScope::Nested(owner) => owner.as_str(),
    };
    let tool_name = event.tool_name.as_deref().unwrap_or("tool");
    let tool_id = event
        .tool_call_id
        .as_deref()
        .or(event.turn_id.as_deref())
        .unwrap_or(tool_name);
    format!("{owner}:{tool_name}:{tool_id}")
}

fn task_activity_key(scope: &AssistantScope, event: &AssistantEvent) -> Option<String> {
    let owner = match scope {
        AssistantScope::Root => event.provider_session_id.as_deref().unwrap_or("root"),
        AssistantScope::Nested(owner) => owner.as_str(),
    };
    event
        .task_id
        .as_deref()
        .map(|task_id| format!("{owner}:{task_id}"))
}

fn clear_pending_completion(session_state: &mut SessionState) {
    session_state.pending_completion = None;
}

fn clear_attention(session_state: &mut SessionState) {
    session_state.attention_pending = false;
    session_state.blocked_reason = None;
}

fn clear_activity_owner(session_state: &mut SessionState, owner: &str) {
    session_state.active_subagents.remove(owner);
    session_state
        .active_tasks
        .retain(|task| !task.starts_with(&format!("{owner}:")));
    session_state
        .active_tools
        .retain(|tool| !tool.starts_with(&format!("{owner}:")));
}

fn should_ignore_root_completion(session_state: &SessionState, event: &AssistantEvent) -> bool {
    let Some(turn_id) = event.turn_id.as_deref() else {
        return false;
    };
    if session_state.completed_turn_id.as_deref() == Some(turn_id) {
        return true;
    }
    if session_state
        .pending_completion
        .as_ref()
        .and_then(|pending| pending.turn_id.as_deref())
        == Some(turn_id)
    {
        return true;
    }
    if let Some(active_turn_id) = session_state.active_turn_id.as_deref() {
        return active_turn_id != turn_id;
    }
    false
}

fn completion_settle_window(provider: AssistantProvider) -> TimeDuration {
    match provider {
        AssistantProvider::Codex => CODEX_COMPLETION_SETTLE_WINDOW,
        AssistantProvider::Claude => CLAUDE_COMPLETION_SETTLE_WINDOW,
    }
}

pub(crate) fn sanitize_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "shell".to_string();
    }
    if let Some(provider) = resolve_assistant_provider(trimmed) {
        return provider.command_name().to_string();
    }

    trimmed.chars().take(200).collect()
}

pub(crate) fn resolve_assistant_provider(command: &str) -> Option<AssistantProvider> {
    let tokens = command
        .split_whitespace()
        .map(|token| token.trim_matches(|ch| ch == '\'' || ch == '"'))
        .collect::<Vec<_>>();

    for window in tokens.windows(3) {
        let command_token = window[0]
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if command_token == "assistant-proxy" && window[1] == "--provider" {
            return AssistantProvider::parse(&window[2].to_ascii_lowercase());
        }
    }

    let token = tokens
        .first()
        .copied()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if token == "silo" {
        return tokens
            .get(1)
            .and_then(|value| AssistantProvider::parse(&value.to_ascii_lowercase()));
    }

    match token.as_str() {
        "codex" => Some(AssistantProvider::Codex),
        "claude" | "cc" => Some(AssistantProvider::Claude),
        _ => None,
    }
}
