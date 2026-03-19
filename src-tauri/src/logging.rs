use crate::state_paths;
use log::LevelFilter;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{plugin::TauriPlugin, Runtime};
use tauri_plugin_log::{
    Builder, RotationStrategy, Target, TargetKind, TimezoneStrategy, WEBVIEW_TARGET,
};
use time::{format_description::FormatItem, macros::format_description, OffsetDateTime};

const SESSION_FILE_STEM_FORMAT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]-[minute]-[second].[subsecond digits:3]");
const SESSION_LOG_MAX_FILE_SIZE: u128 = 10 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct SessionLog {
    pub(crate) directory: PathBuf,
    pub(crate) file_stem: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn build_plugin<R: Runtime>() -> (TauriPlugin<R>, Option<SessionLog>) {
    let session = match create_session_log() {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!("failed to prepare session log directory: {error}");
            None
        }
    };

    let mut builder = Builder::new()
        .clear_targets()
        .level(LevelFilter::Info)
        .level_for("silo_lib", LevelFilter::Debug)
        .level_for(WEBVIEW_TARGET, LevelFilter::Debug)
        .rotation_strategy(RotationStrategy::KeepAll)
        .max_file_size(SESSION_LOG_MAX_FILE_SIZE)
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .target(Target::new(TargetKind::Stdout));

    if let Some(session_log) = &session {
        builder = builder.target(Target::new(TargetKind::Folder {
            path: session_log.directory.clone(),
            file_name: Some(session_log.file_stem.clone()),
        }));
    }

    (builder.build(), session)
}

fn create_session_log() -> Result<SessionLog, String> {
    create_session_log_for_state_dir(state_paths::app_state_dir()?)
}

fn create_session_log_for_state_dir(state_dir: PathBuf) -> Result<SessionLog, String> {
    let directory = session_log_dir(&state_dir);
    ensure_private_dir(&directory)?;

    let local_now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let file_stem = local_now
        .format(SESSION_FILE_STEM_FORMAT)
        .map_err(|error| format!("failed to format log timestamp: {error}"))?;
    let path = directory.join(format!("{file_stem}.log"));

    Ok(SessionLog {
        directory,
        file_stem,
        path,
    })
}

fn session_log_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("logs")
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
            .map_err(|error| format!("failed to create directory {}: {error}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_log_path_uses_silo_logs_directory() {
        let home_dir = std::env::temp_dir().join("silo-logging-test-home");
        let state_dir = home_dir.join(".silo");
        let session = create_session_log_for_state_dir(state_dir.clone())
            .expect("session log should be created");
        assert!(session.directory.ends_with(".silo/logs"));
        assert_eq!(
            session.path,
            session.directory.join(format!("{}.log", session.file_stem))
        );
        assert!(!session.file_stem.contains(':'));

        let _ = fs::remove_dir_all(home_dir);
    }

    #[test]
    fn session_log_dir_is_under_silo_logs() {
        assert_eq!(
            session_log_dir(Path::new("/Users/tester/.silo")),
            PathBuf::from("/Users/tester/.silo/logs")
        );
    }
}
