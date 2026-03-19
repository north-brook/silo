pub(crate) mod state;
pub(crate) mod zmx;

use std::thread;
use std::time::Duration;

use time::OffsetDateTime;

use crate::args::required_flag_value;
use crate::files::observed_file_state;
use crate::metadata::ComputeMetadataClient;
use crate::runtime::{
    acquire_pidfile, ensure_fifo, load_state, persist_state, spawn_fifo_reader, RuntimePaths,
};

use self::state::{
    apply_event, build_published_state, effective_activity_marker,
    should_suspend_for_inactivity_at, ObserverState,
};
use self::zmx::{list_zmx_sessions, read_workspace_branch};

const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(crate) struct DaemonOptions {
    instance: String,
    project: String,
    zone: String,
}

impl DaemonOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        Ok(Self {
            instance: required_flag_value(args, "--instance")?.to_string(),
            project: required_flag_value(args, "--project")?.to_string(),
            zone: required_flag_value(args, "--zone")?.to_string(),
        })
    }
}

pub(crate) fn run_daemon(args: &[String]) -> Result<(), String> {
    let options = DaemonOptions::parse(args)?;
    let runtime = RuntimePaths::new();
    runtime.ensure()?;
    if !acquire_pidfile(&runtime.pidfile)? {
        return Ok(());
    }

    ensure_fifo(&runtime.fifo)?;
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    spawn_fifo_reader(runtime.fifo.clone(), event_tx.clone());

    let mut state = load_state(&runtime.state_file).unwrap_or_default();
    let mut last_published = None::<String>;
    let mut suspend_requested_for_activity = None::<String>;
    let metadata = ComputeMetadataClient::new(
        options.project.clone(),
        options.zone.clone(),
        options.instance.clone(),
    );

    loop {
        while let Ok(event) = event_rx.try_recv() {
            apply_event(&mut state, event);
        }

        if let Err(error) = reconcile_live_sessions(&mut state) {
            eprintln!("workspace-agent: failed to list zmx sessions: {error}");
        }
        reconcile_watched_files(&mut state);
        state.branch = read_workspace_branch();
        persist_state(&runtime.state_file, &state)?;

        let published = build_published_state(&state);
        let published_json =
            serde_json::to_string(&published).map_err(|error| error.to_string())?;

        if last_published.as_deref() != Some(published_json.as_str()) {
            if let Err(error) = metadata.publish(&published) {
                eprintln!("workspace-agent: failed to publish metadata: {error}");
            } else {
                last_published = Some(published_json);
            }
        }

        let effective_activity = effective_activity_marker(&state);
        let should_suspend =
            should_suspend_for_inactivity_at(&state, OffsetDateTime::now_utc(), published.working);
        if !should_suspend {
            suspend_requested_for_activity = None;
        } else if suspend_requested_for_activity.as_deref() != effective_activity.as_deref() {
            match metadata.suspend() {
                Ok(()) => suspend_requested_for_activity = effective_activity,
                Err(error) => {
                    eprintln!("workspace-agent: failed to suspend instance: {error}");
                }
            }
        }

        thread::sleep(POLL_INTERVAL);
    }
}

fn reconcile_live_sessions(state: &mut ObserverState) -> Result<(), String> {
    let live_sessions = list_zmx_sessions()?;
    state::reconcile_sessions(state, &live_sessions);
    Ok(())
}

fn reconcile_watched_files(state: &mut ObserverState) {
    let paths = state.files.watch_paths.iter().cloned().collect::<Vec<_>>();
    state
        .files
        .watched
        .retain(|path, _| state.files.watch_paths.contains(path));

    for path in paths {
        match observed_file_state(&path) {
            Ok(file) => {
                state.files.watched.insert(path, file);
            }
            Err(error) => {
                eprintln!("workspace-agent: failed to observe file state for {path}: {error}");
            }
        }
    }
}
