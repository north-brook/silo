use crate::config::{ConfigStore, ProjectConfig, SiloConfig};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

const PROJECT_IMAGE_CANDIDATES: &[&str] = &[
    "favicon.ico",
    "favicon.png",
    "favicon.svg",
    "public/favicon.ico",
    "public/favicon.png",
    "public/favicon.svg",
    "app/icon.ico",
    "app/icon.png",
    "app/icon.svg",
];

const IMAGE_SEARCH_MAX_SUBDIR_DEPTH: usize = 2;
const IMAGE_SEARCH_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".next",
    ".silo",
    ".turbo",
    "artifacts",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "out",
    "target",
    "vendor",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListedProject {
    name: String,
    path: String,
    image: Option<String>,
}

#[tauri::command]
pub fn projects_list_projects() -> Result<Vec<ListedProject>, String> {
    log::debug!("listing configured projects");
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;

    Ok(listed_projects(&config))
}

#[tauri::command]
pub fn projects_add_project(path: String) -> Result<(), String> {
    log::info!("adding project from path {}", path);
    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    let mut config = store.load().map_err(|error| error.to_string())?;
    let project_root = resolve_project_root(Path::new(&path))?;
    let name = project_key_from_root(&project_root)?;
    let image = resolve_project_image_path(&project_root);

    if config.projects.contains_key(&name) {
        return Err(format!("project already exists: {name}"));
    }

    config.projects.insert(
        name.clone(),
        ProjectConfig {
            name,
            path: project_root.to_string_lossy().into_owned(),
            image,
            gcloud: Default::default(),
        },
    );
    store.save(&config).map_err(|error| error.to_string())?;
    log::info!("project added successfully");
    Ok(())
}

#[tauri::command]
pub fn projects_update_project(
    name: String,
    path: String,
    image: Option<String>,
) -> Result<(), String> {
    log::info!("updating project {name}");
    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    let mut config = store.load().map_err(|error| error.to_string())?;

    let project = config
        .projects
        .get_mut(&name)
        .ok_or_else(|| format!("project not found: {name}"))?;
    project.name = name.clone();
    project.path = path;
    project.image = image;

    store.save(&config).map_err(|error| error.to_string())?;
    log::info!("project {name} updated");
    Ok(())
}

#[tauri::command]
pub fn projects_reorder_projects(project_names: Vec<String>) -> Result<(), String> {
    log::info!("reordering {} projects", project_names.len());
    let store = ConfigStore::new().map_err(|error| error.to_string())?;
    let mut config = store.load().map_err(|error| error.to_string())?;

    validate_project_order(&config, &project_names)?;
    config.projects = reorder_projects(&config.projects, &project_names)?;
    store.save(&config).map_err(|error| error.to_string())?;
    log::info!("project order updated");
    Ok(())
}

fn listed_projects(config: &SiloConfig) -> Vec<ListedProject> {
    config
        .projects
        .iter()
        .map(|(_key, project)| ListedProject {
            name: project.name.clone(),
            path: project.path.clone(),
            image: project.image.clone(),
        })
        .collect()
}

fn validate_project_order(config: &SiloConfig, project_names: &[String]) -> Result<(), String> {
    if project_names.len() != config.projects.len() {
        return Err("project reorder must include every project exactly once".to_string());
    }

    let mut seen = HashSet::new();
    for name in project_names {
        if !config.projects.contains_key(name) {
            return Err(format!("project not found: {name}"));
        }

        if !seen.insert(name.clone()) {
            return Err(format!("duplicate project in reorder list: {name}"));
        }
    }

    Ok(())
}

fn reorder_projects(
    projects: &IndexMap<String, ProjectConfig>,
    project_names: &[String],
) -> Result<IndexMap<String, ProjectConfig>, String> {
    let mut reordered = IndexMap::with_capacity(projects.len());
    for name in project_names {
        let project = projects
            .get(name)
            .cloned()
            .ok_or_else(|| format!("project not found: {name}"))?;
        reordered.insert(name.clone(), project);
    }
    Ok(reordered)
}

fn resolve_project_root(path: &Path) -> Result<PathBuf, String> {
    let resolved = if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
        path.parent()
            .ok_or_else(|| "git directory path must have a parent repository".to_string())?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };

    if !resolved.is_dir() {
        return Err(format!(
            "project path must be an existing directory: {}",
            resolved.display()
        ));
    }

    let git_dir = resolved.join(".git");
    if !(git_dir.is_dir() || git_dir.is_file()) {
        return Err(format!(
            "project path must point to a git repository root: {}",
            resolved.display()
        ));
    }

    Ok(resolved)
}

fn project_key_from_root(project_root: &Path) -> Result<String, String> {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "unable to derive project name from path: {}",
                project_root.display()
            )
        })
}

fn resolve_project_image_path(project_root: &Path) -> Option<String> {
    find_existing_candidate(project_root, PROJECT_IMAGE_CANDIDATES)
        .or_else(|| find_nested_project_image_path(project_root))
        .or_else(|| resolve_package_json_icon(project_root))
        .map(|path| path.to_string_lossy().into_owned())
}

fn find_existing_candidate(root: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|candidate| root.join(candidate))
        .find(|candidate_path| candidate_path.is_file())
}

fn find_nested_project_image_path(project_root: &Path) -> Option<PathBuf> {
    for candidate_root in collect_nested_candidate_roots(project_root) {
        if let Some(path) = find_existing_candidate(&candidate_root, PROJECT_IMAGE_CANDIDATES) {
            return Some(path);
        }
    }

    None
}

fn collect_nested_candidate_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut queue = VecDeque::from([(project_root.to_path_buf(), 0usize)]);

    while let Some((dir_path, depth)) = queue.pop_front() {
        if depth >= IMAGE_SEARCH_MAX_SUBDIR_DEPTH {
            continue;
        }

        let Ok(entries) = fs::read_dir(&dir_path) else {
            continue;
        };

        let mut directories: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if !file_type.is_dir() {
                    return None;
                }

                let name = entry.file_name();
                let name = name.to_str()?;
                (!should_skip_image_search_dir(name)).then(|| entry.path())
            })
            .collect();
        directories.sort();

        for child_path in directories {
            roots.push(child_path.clone());
            queue.push_back((child_path, depth + 1));
        }
    }

    roots
}

fn should_skip_image_search_dir(name: &str) -> bool {
    name.starts_with('.') || IMAGE_SEARCH_EXCLUDED_DIRS.contains(&name)
}

fn resolve_package_json_icon(project_root: &Path) -> Option<PathBuf> {
    let package_json_path = project_root.join("package.json");
    let contents = fs::read_to_string(package_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&contents).ok()?;
    let icon_path = parsed.get("icon")?.as_str()?.trim();
    if icon_path.is_empty() {
        return None;
    }

    let resolved = Path::new(icon_path);
    let resolved = if resolved.is_absolute() {
        resolved.to_path_buf()
    } else {
        project_root.join(resolved)
    };

    resolved.is_file().then_some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ClaudeConfig, CodexConfig, GcloudConfig, GhConfig, ProjectConfig, SiloConfig,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn list_projects_uses_saved_order() {
        let config = SiloConfig {
            gcloud: GcloudConfig::default(),
            gh: GhConfig::default(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            projects: IndexMap::from_iter([
                (
                    "beta".to_string(),
                    ProjectConfig {
                        name: "Beta".to_string(),
                        path: "/tmp/beta".to_string(),
                        image: Some("/tmp/beta.png".to_string()),
                        gcloud: Default::default(),
                    },
                ),
                (
                    "alpha".to_string(),
                    ProjectConfig {
                        name: "Alpha".to_string(),
                        path: "/tmp/alpha".to_string(),
                        image: None,
                        gcloud: Default::default(),
                    },
                ),
            ]),
        };
        let listed = listed_projects(&config);
        let names: Vec<_> = listed.into_iter().map(|project| project.name).collect();
        assert_eq!(names, vec!["Beta".to_string(), "Alpha".to_string()]);
    }

    #[test]
    fn reorder_projects_updates_saved_order() {
        let config = SiloConfig {
            gcloud: GcloudConfig::default(),
            gh: GhConfig::default(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            projects: IndexMap::from_iter([
                (
                    "alpha".to_string(),
                    ProjectConfig {
                        name: "Alpha".to_string(),
                        path: "/tmp/alpha".to_string(),
                        image: None,
                        gcloud: Default::default(),
                    },
                ),
                (
                    "beta".to_string(),
                    ProjectConfig {
                        name: "Beta".to_string(),
                        path: "/tmp/beta".to_string(),
                        image: None,
                        gcloud: Default::default(),
                    },
                ),
            ]),
        };
        let reordered =
            reorder_projects(&config.projects, &["beta".to_string(), "alpha".to_string()])
                .expect("reorder should succeed");
        let names: Vec<_> = reordered.keys().cloned().collect();
        assert_eq!(names, vec!["beta".to_string(), "alpha".to_string()]);
    }

    #[test]
    fn reorder_projects_rejects_unknown_or_duplicate_names() {
        let config = SiloConfig {
            gcloud: GcloudConfig::default(),
            gh: GhConfig::default(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            projects: IndexMap::from_iter([
                (
                    "alpha".to_string(),
                    ProjectConfig {
                        name: "Alpha".to_string(),
                        path: "/tmp/alpha".to_string(),
                        image: None,
                        gcloud: Default::default(),
                    },
                ),
                (
                    "beta".to_string(),
                    ProjectConfig {
                        name: "Beta".to_string(),
                        path: "/tmp/beta".to_string(),
                        image: None,
                        gcloud: Default::default(),
                    },
                ),
            ]),
        };

        assert!(
            validate_project_order(&config, &["alpha".to_string(), "alpha".to_string()]).is_err()
        );
        assert!(
            validate_project_order(&config, &["alpha".to_string(), "gamma".to_string()]).is_err()
        );
    }

    #[test]
    fn add_project_derives_name_and_image_from_repo_root() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join(".git")).expect("git dir should be created");
        fs::write(project_root.join("favicon.png"), b"png").expect("favicon should be written");

        let root = resolve_project_root(&project_root).expect("project root should resolve");
        let name = project_key_from_root(&root).expect("name should resolve");
        let image = resolve_project_image_path(&root).expect("image should resolve");

        assert_eq!(name, "demo");
        assert_eq!(image, project_root.join("favicon.png").to_string_lossy());
    }

    #[test]
    fn add_project_accepts_dot_git_path() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join(".git")).expect("git dir should be created");

        let root =
            resolve_project_root(&project_root.join(".git")).expect("project root should resolve");
        assert_eq!(root, project_root);
    }

    #[test]
    fn resolves_package_json_icon_when_no_direct_icon_exists() {
        let temp_dir = TempDir::new();
        let project_root = temp_dir.path.join("demo");
        fs::create_dir_all(project_root.join(".git")).expect("git dir should be created");
        fs::create_dir_all(project_root.join("assets")).expect("assets dir should be created");
        fs::write(project_root.join("assets/icon.png"), b"png").expect("icon should be written");
        fs::write(
            project_root.join("package.json"),
            r#"{"icon":"assets/icon.png"}"#,
        )
        .expect("package.json should be written");

        let image = resolve_project_image_path(&project_root).expect("image should resolve");
        assert_eq!(
            image,
            project_root.join("assets/icon.png").to_string_lossy()
        );
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = format!(
                "silo-project-image-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
