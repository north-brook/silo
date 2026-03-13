use crate::config::ConfigStore;
use std::path::{Path, PathBuf};
use std::process::Command;

const MACOS_CHROME_USER_DATA_DIR: &str = "Library/Application Support/Google/Chrome";
const MACOS_CHROME_APP_PATH: &str = "/Applications/Google Chrome.app";

pub(crate) fn detect_chrome_user_data_dir(home_dir: &Path) -> Option<String> {
    let path = home_dir.join(MACOS_CHROME_USER_DATA_DIR);
    if path.is_dir() {
        return Some(path.to_string_lossy().into_owned());
    }

    None
}

#[tauri::command]
pub async fn chrome_installed() -> bool {
    log::trace!("checking whether google-chrome is installed");
    tauri::async_runtime::spawn_blocking(move || chrome_installed_on_host())
        .await
        .unwrap_or(false)
}

#[tauri::command]
pub async fn chrome_configured() -> bool {
    log::trace!("checking whether chrome is configured");
    ConfigStore::new()
        .and_then(|store| store.load())
        .map(|config| chrome_source_dir_exists(&config.chrome.user_data_dir))
        .unwrap_or(false)
}

pub(crate) fn chrome_source_dir_exists(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }

    PathBuf::from(trimmed).is_dir()
}

fn chrome_installed_on_host() -> bool {
    if cfg!(target_os = "macos") {
        return Path::new(MACOS_CHROME_APP_PATH).is_dir();
    }

    Command::new("google-chrome")
        .arg("--version")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        chrome_installed_on_host, chrome_source_dir_exists, detect_chrome_user_data_dir,
        MACOS_CHROME_APP_PATH,
    };
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn detect_chrome_user_data_dir_returns_default_macos_path() {
        let temp_dir = TestDir::new();
        let chrome_dir = temp_dir
            .root
            .join("Library/Application Support/Google/Chrome");
        fs::create_dir_all(&chrome_dir).expect("chrome dir should be created");

        let detected =
            detect_chrome_user_data_dir(&temp_dir.root).expect("chrome dir should be detected");
        assert_eq!(detected, chrome_dir.to_string_lossy());
    }

    #[test]
    fn detect_chrome_user_data_dir_returns_none_when_missing() {
        let temp_dir = TestDir::new();
        assert!(detect_chrome_user_data_dir(&temp_dir.root).is_none());
    }

    #[test]
    fn chrome_source_dir_exists_requires_a_directory() {
        let temp_dir = TestDir::new();
        let chrome_dir = temp_dir.root.join("Chrome");
        fs::create_dir_all(&chrome_dir).expect("chrome dir should be created");

        assert!(chrome_source_dir_exists(&chrome_dir.to_string_lossy()));
        assert!(!chrome_source_dir_exists(""));
        assert!(!chrome_source_dir_exists("/tmp/does-not-exist"));
    }

    #[test]
    fn chrome_installed_on_host_matches_platform_check() {
        if cfg!(target_os = "macos") {
            assert_eq!(
                chrome_installed_on_host(),
                Path::new(MACOS_CHROME_APP_PATH).is_dir()
            );
        }
    }

    struct TestDir {
        root: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "silo-chrome-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let root = env::temp_dir().join(unique);
            fs::create_dir_all(&root).expect("test dir should be created");

            Self { root }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
