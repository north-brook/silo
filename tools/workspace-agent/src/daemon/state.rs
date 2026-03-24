use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};

use super::zmx::ZmxSession;

const HEARTBEAT_PUBLISH_INTERVAL: TimeDuration = TimeDuration::seconds(15);
const AUTO_SUSPEND_IDLE_THRESHOLD: TimeDuration = TimeDuration::hours(4);
const POLL_MISS_THRESHOLD_UNMANAGED: u16 = 3;
pub(crate) const POLL_MISS_THRESHOLD_LIFECYCLE: u16 = 300;

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
    },
    AssistantTurnCompleted {
        session: String,
        provider: AssistantProvider,
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
    touch_last_active(state);
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
            session_state.command_running = true;
            session_state.working = false;
            session_state.unread = false;
            session_state.poll_misses = 0;
        }
        ObserverEvent::ShellCommandFinished { session } => {
            if let Some(session_state) = state.sessions.get_mut(&session) {
                session_state.active_command = None;
                session_state.assistant_provider = None;
                session_state.command_running = false;
                session_state.working = false;
                session_state.unread = false;
                session_state.poll_misses = 0;
            }
        }
        ObserverEvent::AssistantSessionStarted { session, provider } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(provider.command_name().to_string());
            session_state.assistant_provider = Some(provider);
            session_state.command_running = true;
            session_state.working = false;
            session_state.unread = false;
            session_state.poll_misses = 0;
        }
        ObserverEvent::AssistantPromptSubmitted { session, provider } => {
            {
                let session_state = state.sessions.entry(session).or_default();
                session_state.active_command = Some(provider.command_name().to_string());
                session_state.assistant_provider = Some(provider);
                session_state.command_running = true;
                session_state.working = true;
                session_state.unread = false;
                session_state.poll_misses = 0;
            }
            touch_last_working(state);
        }
        ObserverEvent::AssistantTurnCompleted { session, provider } => {
            let session_state = state.sessions.entry(session).or_default();
            session_state.active_command = Some(provider.command_name().to_string());
            session_state.assistant_provider = Some(provider);
            session_state.command_running = true;
            session_state.working = false;
            session_state.unread = true;
            session_state.poll_misses = 0;
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
    reconcile_active_session(state);

    if state.sessions != before {
        touch_last_active(state);
    }
}

pub(crate) fn build_published_state(state: &ObserverState) -> PublishedState {
    let mut working = false;
    let mut unread = false;
    let mut terminals = state
        .sessions
        .iter()
        .map(|(session_name, session_state)| {
            let name = session_state
                .active_command
                .clone()
                .unwrap_or_else(|| "shell".to_string());
            let assistant_capable = session_state.assistant_provider.is_some()
                || resolve_assistant_provider(&name).is_some();
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

fn touch_last_active(state: &mut ObserverState) {
    state.last_active = Some(current_timestamp());
}

fn touch_last_working(state: &mut ObserverState) {
    state.last_working = Some(current_timestamp());
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
    if !state.active_session.as_ref().is_some_and(|active| {
        session_exists(state, &active.kind, &active.attachment_id)
    }) {
        state.active_session = None;
    }
}

pub(crate) fn sanitize_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return "shell".to_string();
    }
    if trimmed.eq("cc") || trimmed.starts_with("cc ") {
        return "claude".to_string();
    }
    if trimmed.eq("silo codex") || trimmed.starts_with("silo codex ") {
        return "codex".to_string();
    }
    if trimmed.eq("silo claude") || trimmed.starts_with("silo claude ") {
        return "claude".to_string();
    }

    trimmed.chars().take(200).collect()
}

pub(crate) fn resolve_assistant_provider(command: &str) -> Option<AssistantProvider> {
    let token = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match token.as_str() {
        "codex" => Some(AssistantProvider::Codex),
        "claude" | "cc" => Some(AssistantProvider::Claude),
        _ => None,
    }
}
