use crate::build_info;
use std::env;
use std::path::{Path, PathBuf};

pub(crate) const SILO_STATE_DIR_ENV_VAR: &str = "SILO_STATE_DIR";

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "unable to determine the home directory".to_string())
}

pub(crate) fn app_state_dir() -> Result<PathBuf, String> {
    Ok(app_state_dir_for_home(home_dir()?))
}

pub(crate) fn app_state_dir_for_home(home_dir: impl AsRef<Path>) -> PathBuf {
    env::var_os(SILO_STATE_DIR_ENV_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir.as_ref().join(build_info::default_state_dir_name()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_to_home_silo_directory() {
        let _guard = ENV_LOCK.lock().expect("env lock should be available");
        let previous = env::var_os(SILO_STATE_DIR_ENV_VAR);
        env::remove_var(SILO_STATE_DIR_ENV_VAR);

        assert_eq!(
            app_state_dir_for_home("/Users/tester"),
            PathBuf::from(format!(
                "/Users/tester/{}",
                build_info::default_state_dir_name()
            ))
        );

        if let Some(previous) = previous {
            env::set_var(SILO_STATE_DIR_ENV_VAR, previous);
        }
    }

    #[test]
    fn uses_explicit_state_dir_override_when_present() {
        let _guard = ENV_LOCK.lock().expect("env lock should be available");
        let previous = env::var_os(SILO_STATE_DIR_ENV_VAR);
        env::set_var(SILO_STATE_DIR_ENV_VAR, "/tmp/silo-e2e-state");

        assert_eq!(
            app_state_dir_for_home("/Users/tester"),
            PathBuf::from("/tmp/silo-e2e-state")
        );

        if let Some(previous) = previous {
            env::set_var(SILO_STATE_DIR_ENV_VAR, previous);
        } else {
            env::remove_var(SILO_STATE_DIR_ENV_VAR);
        }
    }
}
