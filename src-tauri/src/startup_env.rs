use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub(crate) struct StartupEnvironmentReport {
    message: Option<String>,
    path: Option<String>,
}

impl StartupEnvironmentReport {
    pub(crate) fn log(&self) {
        if let Some(message) = &self.message {
            log::info!("{message}");
        }

        if let Some(path) = &self.path {
            log::debug!("effective PATH={path}");
        }
    }
}

pub(crate) fn initialize_process_environment() -> StartupEnvironmentReport {
    #[cfg(target_os = "macos")]
    {
        initialize_macos_process_environment()
    }

    #[cfg(not(target_os = "macos"))]
    {
        StartupEnvironmentReport::default()
    }
}

#[cfg(target_os = "macos")]
fn initialize_macos_process_environment() -> StartupEnvironmentReport {
    let current_path = env::var_os("PATH").unwrap_or_default();
    let login_shell_path = read_login_shell_path();
    let path_helper_path = read_path_helper_path();
    let fallback_paths = fallback_path_entries();

    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut sources = Vec::new();

    if push_split_paths(&mut entries, &mut seen, &login_shell_path) {
        sources.push("login shell");
    }

    if push_split_paths(&mut entries, &mut seen, &current_path) {
        sources.push("inherited PATH");
    }

    if push_split_paths(&mut entries, &mut seen, &path_helper_path) {
        sources.push("path_helper");
    }

    if push_path_entries(&mut entries, &mut seen, fallback_paths) {
        sources.push("fallbacks");
    }

    if entries.is_empty() {
        return StartupEnvironmentReport {
            message: Some("startup PATH initialization found no usable entries".to_string()),
            path: None,
        };
    }

    let resolved_path = match env::join_paths(entries.iter()) {
        Ok(path) => path,
        Err(error) => {
            return StartupEnvironmentReport {
                message: Some(format!("startup PATH initialization failed: {error}")),
                path: None,
            };
        }
    };

    if resolved_path != current_path {
        env::set_var("PATH", &resolved_path);
    }

    let source_summary = if sources.is_empty() {
        "existing environment".to_string()
    } else {
        sources.join(", ")
    };

    StartupEnvironmentReport {
        message: Some(format!(
            "initialized process PATH for macOS app launch using {source_summary}"
        )),
        path: Some(resolved_path.to_string_lossy().into_owned()),
    }
}

#[cfg(target_os = "macos")]
fn read_login_shell_path() -> OsString {
    let shell = env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/bin/zsh"));

    let output = match Command::new(shell)
        .args(["-l", "-c", "printenv PATH"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return OsString::new(),
    };

    extract_path_from_shell_output(&String::from_utf8_lossy(&output.stdout)).into()
}

#[cfg(target_os = "macos")]
fn read_path_helper_path() -> OsString {
    let output = match Command::new("/usr/libexec/path_helper").arg("-s").output() {
        Ok(output) if output.status.success() => output,
        _ => return OsString::new(),
    };

    parse_path_helper_output(&String::from_utf8_lossy(&output.stdout)).into()
}

#[cfg(target_os = "macos")]
fn extract_path_from_shell_output(output: &str) -> String {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.contains('/'))
        .unwrap_or_default()
        .to_string()
}

#[cfg(target_os = "macos")]
fn parse_path_helper_output(output: &str) -> String {
    let trimmed = output.trim();
    let Some(start) = trimmed.find("PATH=") else {
        return String::new();
    };

    let mut value = &trimmed[start + "PATH=".len()..];
    if let Some(stripped) = value.strip_prefix('"') {
        let Some(end) = stripped.find('"') else {
            return String::new();
        };
        return stripped[..end].to_string();
    }

    if let Some(end) = value.find(';') {
        value = &value[..end];
    }

    value.trim().to_string()
}

#[cfg(target_os = "macos")]
fn fallback_path_entries() -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let home_dir = env::var_os("HOME").map(PathBuf::from);

    for entry in [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/local/sbin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ] {
        if entry.is_dir() {
            entries.push(entry);
        }
    }

    if let Some(home_dir) = home_dir {
        for entry in [
            home_dir.join(".cargo/bin"),
            home_dir.join(".bun/bin"),
            home_dir.join(".local/bin"),
        ] {
            if entry.is_dir() {
                entries.push(entry);
            }
        }
    }

    entries
}

#[cfg(target_os = "macos")]
fn push_split_paths(
    entries: &mut Vec<PathBuf>,
    seen: &mut HashSet<OsString>,
    value: &OsString,
) -> bool {
    if value.is_empty() {
        return false;
    }

    let mut added = false;
    for path in env::split_paths(value) {
        let key = normalize_path_key(&path);
        if seen.insert(key) {
            entries.push(path);
            added = true;
        }
    }
    added
}

#[cfg(target_os = "macos")]
fn push_path_entries(
    entries: &mut Vec<PathBuf>,
    seen: &mut HashSet<OsString>,
    candidates: Vec<PathBuf>,
) -> bool {
    let mut added = false;
    for path in candidates {
        let key = normalize_path_key(&path);
        if seen.insert(key) {
            entries.push(path);
            added = true;
        }
    }
    added
}

#[cfg(target_os = "macos")]
fn normalize_path_key(path: &Path) -> OsString {
    if let Ok(canonical) = path.canonicalize() {
        canonical.into_os_string()
    } else {
        path.as_os_str().to_os_string()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::{
        extract_path_from_shell_output, normalize_path_key, parse_path_helper_output,
        push_split_paths,
    };
    #[cfg(target_os = "macos")]
    use std::collections::HashSet;
    #[cfg(target_os = "macos")]
    use std::env;
    #[cfg(target_os = "macos")]
    use std::ffi::OsString;
    #[cfg(target_os = "macos")]
    use std::path::PathBuf;

    #[cfg(target_os = "macos")]
    #[test]
    fn extracts_path_from_last_shell_output_line() {
        let output = "sourcing profile\n/opt/homebrew/bin:/usr/bin:/bin\n";
        assert_eq!(
            extract_path_from_shell_output(output),
            "/opt/homebrew/bin:/usr/bin:/bin"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_path_helper_output() {
        let output = "PATH=\"/opt/homebrew/bin:/usr/bin:/bin\"; export PATH;";
        assert_eq!(
            parse_path_helper_output(output),
            "/opt/homebrew/bin:/usr/bin:/bin"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn split_path_push_preserves_order_and_deduplicates() {
        let mut entries = Vec::<PathBuf>::new();
        let mut seen = HashSet::<OsString>::new();

        assert!(push_split_paths(
            &mut entries,
            &mut seen,
            &OsString::from("/opt/homebrew/bin:/usr/bin:/opt/homebrew/bin")
        ));

        let joined = env::join_paths(entries.iter()).expect("paths should join");
        assert_eq!(joined.to_string_lossy(), "/opt/homebrew/bin:/usr/bin");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn normalize_path_key_resolves_symlinks_when_possible() {
        let key = normalize_path_key(&PathBuf::from("/bin"));
        assert!(!key.is_empty());
    }
}
