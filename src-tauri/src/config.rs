#![allow(dead_code)]

use crate::codex::detect_codex_token;
use crate::state_paths;
use indexmap::IndexMap;
use log::{info, trace};
use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use toml::{Table, Value};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

const SILO_DIR_NAME: &str = ".silo";
const CONFIG_FILE_NAME: &str = "config.toml";
const SERVICE_ACCOUNT_KEY_SUFFIX: &str = "-silo-workspaces.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct SiloConfig {
    pub(crate) gcloud: GcloudConfig,
    pub(crate) git: GitConfig,
    pub(crate) codex: CodexConfig,
    pub(crate) claude: ClaudeConfig,
    pub(crate) projects: IndexMap<String, ProjectConfig>,
}

pub(crate) const DEFAULT_GCLOUD_REGION: &str = "us-east4";
pub(crate) const DEFAULT_GCLOUD_ZONE: &str = "us-east4-c";
pub(crate) const DEFAULT_GCLOUD_MACHINE_TYPE: &str = "e2-standard-4";
pub(crate) const DEFAULT_GCLOUD_DISK_SIZE_GB: u32 = 80;
pub(crate) const DEFAULT_GCLOUD_DISK_TYPE: &str = "pd-ssd";
pub(crate) const DEFAULT_GCLOUD_IMAGE_FAMILY: &str = "silo-base";
pub(crate) const DEFAULT_GCLOUD_IMAGE_PROJECT: &str = "silo-489618";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct GcloudConfig {
    pub(crate) account: String,
    pub(crate) service_account: String,
    pub(crate) service_account_key_file: String,
    pub(crate) project: String,
    pub(crate) region: String,
    pub(crate) zone: String,
    pub(crate) machine_type: String,
    pub(crate) disk_size_gb: u32,
    pub(crate) disk_type: String,
    pub(crate) image_family: String,
    pub(crate) image_project: String,
}

impl Default for GcloudConfig {
    fn default() -> Self {
        Self {
            account: String::new(),
            service_account: String::new(),
            service_account_key_file: String::new(),
            project: String::new(),
            region: DEFAULT_GCLOUD_REGION.to_string(),
            zone: DEFAULT_GCLOUD_ZONE.to_string(),
            machine_type: DEFAULT_GCLOUD_MACHINE_TYPE.to_string(),
            disk_size_gb: DEFAULT_GCLOUD_DISK_SIZE_GB,
            disk_type: DEFAULT_GCLOUD_DISK_TYPE.to_string(),
            image_family: DEFAULT_GCLOUD_IMAGE_FAMILY.to_string(),
            image_project: DEFAULT_GCLOUD_IMAGE_PROJECT.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ProjectGcloudConfig {
    pub(crate) account: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) zone: Option<String>,
    pub(crate) machine_type: Option<String>,
    pub(crate) disk_size_gb: Option<u32>,
    pub(crate) disk_type: Option<String>,
    pub(crate) image_family: Option<String>,
    pub(crate) image_project: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct GitConfig {
    pub(crate) gh_username: String,
    pub(crate) gh_token: String,
    pub(crate) user_name: String,
    pub(crate) user_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct CodexConfig {
    pub(crate) token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ClaudeConfig {
    pub(crate) token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub(crate) struct ProjectConfig {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) image: Option<String>,
    pub(crate) remote_url: String,
    pub(crate) target_branch: String,
    pub(crate) env_files: Vec<String>,
    pub(crate) gcloud: ProjectGcloudConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct ConfigStore {
    home_dir: PathBuf,
    silo_dir: PathBuf,
    config_path: PathBuf,
}

impl ConfigStore {
    pub(crate) fn new() -> Result<Self, ConfigError> {
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(ConfigError::HomeDirectoryNotFound)?;

        Ok(Self::from_home_dir(home_dir))
    }

    pub(crate) fn from_home_dir(home_dir: PathBuf) -> Self {
        let silo_dir = state_paths::app_state_dir_for_home(&home_dir);
        let config_path = silo_dir.join(CONFIG_FILE_NAME);

        Self {
            home_dir,
            silo_dir,
            config_path,
        }
    }

    pub(crate) fn initialize_defaults_if_missing(&self) -> Result<(), ConfigError> {
        let _guard = CONFIG_LOCK.lock().map_err(|_| ConfigError::LockPoisoned)?;
        trace!("ensuring config exists at {}", self.config_path.display());
        self.initialize_defaults_if_missing_locked(|home_dir| detect_initial_config(home_dir))
    }

    pub(crate) fn load(&self) -> Result<SiloConfig, ConfigError> {
        let _guard = CONFIG_LOCK.lock().map_err(|_| ConfigError::LockPoisoned)?;
        trace!("loading config from {}", self.config_path.display());
        self.load_locked()
    }

    pub(crate) fn read(&self, path: &str) -> Result<Value, ConfigError> {
        let _guard = CONFIG_LOCK.lock().map_err(|_| ConfigError::LockPoisoned)?;
        trace!("reading config path {path}");
        let config = self.load_locked()?;
        let value = config_to_value(&config)?;
        let segments = parse_path(path)?;

        read_value_at_path(&value, &segments)
    }

    pub(crate) fn write(&self, path: &str, value: Value) -> Result<(), ConfigError> {
        let _guard = CONFIG_LOCK.lock().map_err(|_| ConfigError::LockPoisoned)?;
        trace!("writing config path {path}");
        let config = self.load_locked()?;
        let mut root = config_to_value(&config)?;
        let segments = parse_path(path)?;

        write_value_at_path(&mut root, &segments, value)?;

        let next_config: SiloConfig = root
            .try_into()
            .map_err(|error| ConfigError::Schema(error.to_string()))?;

        self.save_locked(&next_config)
    }

    pub(crate) fn save(&self, config: &SiloConfig) -> Result<(), ConfigError> {
        let _guard = CONFIG_LOCK.lock().map_err(|_| ConfigError::LockPoisoned)?;
        trace!("saving config to {}", self.config_path.display());
        self.save_locked(config)
    }

    fn load_locked(&self) -> Result<SiloConfig, ConfigError> {
        self.ensure_config_file_locked()?;

        let contents = fs::read_to_string(&self.config_path).map_err(ConfigError::Io)?;
        let config: SiloConfig = toml::from_str(&contents).map_err(ConfigError::Parse)?;
        validate_config(&config)?;
        Ok(config)
    }

    fn save_locked(&self, config: &SiloConfig) -> Result<(), ConfigError> {
        self.ensure_silo_dir()?;
        let contents = serialize_config(config)?;
        self.write_atomically(&contents)
    }

    fn ensure_config_file_locked(&self) -> Result<(), ConfigError> {
        self.initialize_defaults_if_missing_locked(|home_dir| detect_initial_config(home_dir))
    }

    fn ensure_silo_dir(&self) -> Result<(), ConfigError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;

            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(&self.silo_dir).map_err(ConfigError::Io)?;
            ensure_unix_permissions(&self.silo_dir, 0o700)?;
        }

        #[cfg(not(unix))]
        {
            fs::create_dir_all(&self.silo_dir).map_err(ConfigError::Io)?;
        }

        Ok(())
    }

    fn write_atomically(&self, contents: &str) -> Result<(), ConfigError> {
        self.ensure_silo_dir()?;

        let temp_path = self.temp_path();
        write_file(&temp_path, contents)?;

        #[cfg(windows)]
        if self.config_path.exists() {
            fs::remove_file(&self.config_path).map_err(ConfigError::Io)?;
        }

        fs::rename(&temp_path, &self.config_path).map_err(ConfigError::Io)?;

        #[cfg(unix)]
        ensure_unix_permissions(&self.config_path, 0o600)?;

        Ok(())
    }

    fn temp_path(&self) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);

        self.silo_dir.join(format!(
            "{CONFIG_FILE_NAME}.tmp.{}.{}",
            std::process::id(),
            nanos
        ))
    }

    fn initialize_defaults_if_missing_locked<F>(&self, detect: F) -> Result<(), ConfigError>
    where
        F: FnOnce(&Path) -> SiloConfig,
    {
        self.ensure_silo_dir()?;

        if self.config_path.exists() {
            #[cfg(unix)]
            ensure_unix_permissions(&self.config_path, 0o600)?;
            return Ok(());
        }

        info!("creating initial config at {}", self.config_path.display());
        let config = detect(&self.home_dir);
        self.write_atomically(&serialize_config(&config)?)
    }
}

pub(crate) fn initialize_on_start() -> Result<(), ConfigError> {
    info!("initializing config on startup");
    ConfigStore::new()?.initialize_defaults_if_missing()
}

#[derive(Debug)]
pub(crate) enum ConfigError {
    HomeDirectoryNotFound,
    InvalidPath,
    PathNotFound(String),
    TypeMismatch(String),
    Schema(String),
    Io(std::io::Error),
    Parse(toml::de::Error),
    Serialize(toml::ser::Error),
    LockPoisoned,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeDirectoryNotFound => f.write_str("unable to determine the home directory"),
            Self::InvalidPath => {
                f.write_str("config path must not be empty and cannot contain empty segments")
            }
            Self::PathNotFound(path) => write!(f, "config path not found: {path}"),
            Self::TypeMismatch(path) => {
                write!(f, "config path points at a non-table value: {path}")
            }
            Self::Schema(message) => write!(f, "config value does not match the schema: {message}"),
            Self::Io(error) => write!(f, "config I/O error: {error}"),
            Self::Parse(error) => write!(f, "config parse error: {error}"),
            Self::Serialize(error) => write!(f, "config serialization error: {error}"),
            Self::LockPoisoned => f.write_str("config store lock poisoned"),
        }
    }
}

impl std::error::Error for ConfigError {}

fn parse_path(path: &str) -> Result<Vec<&str>, ConfigError> {
    if path.is_empty() {
        return Err(ConfigError::InvalidPath);
    }

    let segments: Vec<&str> = path.split('.').collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(ConfigError::InvalidPath);
    }

    Ok(segments)
}

fn read_value_at_path(root: &Value, segments: &[&str]) -> Result<Value, ConfigError> {
    let mut current = root;
    let mut traversed = Vec::with_capacity(segments.len());

    for segment in segments {
        traversed.push(*segment);

        let table = current
            .as_table()
            .ok_or_else(|| ConfigError::TypeMismatch(traversed.join(".")))?;

        current = table
            .get(*segment)
            .ok_or_else(|| ConfigError::PathNotFound(traversed.join(".")))?;
    }

    Ok(current.clone())
}

fn write_value_at_path(
    root: &mut Value,
    segments: &[&str],
    new_value: Value,
) -> Result<(), ConfigError> {
    let mut current = root;
    let mut traversed = Vec::with_capacity(segments.len());

    for (index, segment) in segments.iter().enumerate() {
        traversed.push(*segment);

        let table = current
            .as_table_mut()
            .ok_or_else(|| ConfigError::TypeMismatch(traversed.join(".")))?;

        if index == segments.len() - 1 {
            table.insert((*segment).to_string(), new_value);
            return Ok(());
        }

        current = table
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Table(Table::new()));
    }

    Err(ConfigError::InvalidPath)
}

fn config_to_value(config: &SiloConfig) -> Result<Value, ConfigError> {
    let mut value =
        Value::try_from(config).map_err(|error| ConfigError::Schema(error.to_string()))?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| ConfigError::Schema("root config is not a table".to_string()))?;

    table
        .entry("gcloud".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    table
        .entry("git".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    table
        .entry("codex".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    table
        .entry("claude".to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    table
        .entry("projects".to_string())
        .or_insert_with(|| Value::Table(Table::new()));

    Ok(value)
}

fn serialize_config(config: &SiloConfig) -> Result<String, ConfigError> {
    validate_config(config)?;
    let value = config_to_value(config)?;
    toml::to_string_pretty(&value).map_err(ConfigError::Serialize)
}

fn detect_initial_config(home_dir: &Path) -> SiloConfig {
    let mut gcloud = GcloudConfig::default();
    gcloud.account =
        command_output("gcloud", ["config", "get-value", "account"]).unwrap_or_default();
    gcloud.project =
        command_output("gcloud", ["config", "get-value", "project"]).unwrap_or_default();
    if gcloud.account.ends_with(".gserviceaccount.com") {
        gcloud.service_account = gcloud.account.clone();
        gcloud.service_account_key_file =
            detect_service_account_key_file(home_dir, &gcloud.project).unwrap_or_default();
    }

    SiloConfig {
        gcloud,
        git: GitConfig {
            gh_username: command_output("gh", ["api", "user", "--jq", ".login"])
                .unwrap_or_default(),
            gh_token: command_output("gh", ["auth", "token"]).unwrap_or_default(),
            user_name: command_output("git", ["config", "--global", "user.name"])
                .unwrap_or_default(),
            user_email: command_output("git", ["config", "--global", "user.email"])
                .unwrap_or_default(),
        },
        codex: CodexConfig {
            token: detect_codex_token(home_dir).unwrap_or_default(),
        },
        claude: ClaudeConfig::default(),
        projects: IndexMap::new(),
    }
}

fn detect_service_account_key_file(home_dir: &Path, project: &str) -> Option<String> {
    let trimmed = project.trim();
    if trimmed.is_empty() {
        return None;
    }

    let safe_project = trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let path = state_paths::app_state_dir_for_home(home_dir)
        .join(format!("{safe_project}{SERVICE_ACCOUNT_KEY_SUFFIX}"));

    path.is_file().then(|| path.to_string_lossy().into_owned())
}

fn validate_config(config: &SiloConfig) -> Result<(), ConfigError> {
    for (name, project) in &config.projects {
        if project.name.trim().is_empty() {
            return Err(ConfigError::Schema(format!(
                "projects.{name}.name must not be empty"
            )));
        }

        if project.path.trim().is_empty() {
            return Err(ConfigError::Schema(format!(
                "projects.{name}.path must not be empty"
            )));
        }

        if matches!(project.gcloud.disk_size_gb, Some(0)) {
            return Err(ConfigError::Schema(format!(
                "projects.{name}.gcloud.disk_size_gb must be greater than zero"
            )));
        }
    }

    if config.gcloud.disk_size_gb == 0 {
        return Err(ConfigError::Schema(
            "gcloud.disk_size_gb must be greater than zero".to_string(),
        ));
    }

    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<(), ConfigError> {
    let mut file = create_private_file(path)?;
    file.write_all(contents.as_bytes())
        .map_err(ConfigError::Io)?;
    file.sync_all().map_err(ConfigError::Io)?;
    Ok(())
}

fn command_output<I, S>(program: &str, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    normalize_value(String::from_utf8_lossy(&output.stdout).trim())
}

fn normalize_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "(unset)" || trimmed == "unset" {
        return None;
    }

    Some(trimmed.to_owned())
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> Result<File, ConfigError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(ConfigError::Io)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> Result<File, ConfigError> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(ConfigError::Io)
}

#[cfg(unix)]
fn ensure_unix_permissions(path: &Path, mode: u32) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(ConfigError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn first_load_creates_default_config() {
        let temp_dir = TestDir::new_without_config();
        let store = temp_dir.store();

        store
            .initialize_defaults_if_missing_locked(|_| SiloConfig::default())
            .expect("initialization should create default config");

        let config = store.load().expect("load should read initialized config");

        assert_eq!(config, SiloConfig::default());
        assert!(temp_dir.silo_dir().exists());
        assert!(temp_dir.config_path().exists());

        let contents =
            fs::read_to_string(temp_dir.config_path()).expect("config file should be readable");
        assert!(contents.contains("[gcloud]"));
        assert!(contents.contains("[git]"));
        assert!(contents.contains("[codex]"));
        assert!(contents.contains("[claude]"));
        assert!(contents.contains("[projects]"));
    }

    #[test]
    fn read_supports_sections_and_leaf_values() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        store
            .write("claude.token", Value::String("secret".to_string()))
            .expect("write should succeed");

        assert_eq!(
            store
                .read("claude.token")
                .expect("leaf read should succeed"),
            Value::String("secret".to_string())
        );

        let section = store.read("claude").expect("section read should succeed");
        let token = section
            .as_table()
            .and_then(|table| table.get("token"))
            .cloned()
            .expect("claude.token should be present");

        assert_eq!(token, Value::String("secret".to_string()));
    }

    #[test]
    fn write_replaces_subtrees() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        let mut claude = Table::new();
        claude.insert("token".to_string(), Value::String("new-token".to_string()));

        store
            .write("claude", Value::Table(claude))
            .expect("section write should succeed");

        assert_eq!(
            store
                .read("claude.token")
                .expect("leaf read should succeed"),
            Value::String("new-token".to_string())
        );
    }

    #[test]
    fn write_supports_projects_and_leaf_reads() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        let mut project = Table::new();
        project.insert("name".to_string(), Value::String("My Project".to_string()));
        project.insert(
            "path".to_string(),
            Value::String("/tmp/project".to_string()),
        );
        project.insert(
            "image".to_string(),
            Value::String("/tmp/image.png".to_string()),
        );

        store
            .write("projects.myproj", Value::Table(project))
            .expect("project write should succeed");

        assert!(store
            .read("projects")
            .expect("projects read should succeed")
            .is_table());
        assert!(store
            .read("projects.myproj")
            .expect("project read should succeed")
            .is_table());
        assert_eq!(
            store
                .read("projects.myproj.image")
                .expect("project image read should succeed"),
            Value::String("/tmp/image.png".to_string())
        );
    }

    #[test]
    fn write_rejects_schema_breaking_values() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        let error = store
            .write("claude", Value::String("wrong".to_string()))
            .expect_err("invalid section type should fail");

        assert!(matches!(error, ConfigError::Schema(_)));
    }

    #[test]
    fn write_requires_semantically_valid_projects() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        let error = store
            .write(
                "projects.myproj.image",
                Value::String("/tmp/image.png".to_string()),
            )
            .expect_err("image-only project should fail schema validation");

        assert!(matches!(error, ConfigError::Schema(_)));
    }

    #[test]
    fn invalid_paths_and_missing_reads_return_errors() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        let invalid = store.read("").expect_err("empty path should be invalid");
        assert!(matches!(invalid, ConfigError::InvalidPath));

        let missing = store
            .read("projects.unknown")
            .expect_err("unknown project should fail");
        assert!(matches!(missing, ConfigError::PathNotFound(_)));
    }

    #[test]
    fn partial_config_backfills_defaults() {
        let temp_dir = TestDir::new();

        write_file(
            &temp_dir.config_path(),
            "[claude]\ntoken = \"partial\"\n[projects.demo]\nname = \"Demo\"\npath = \"/tmp/demo\"\n",
        )
        .expect("seed config should be written");

        let config = temp_dir.store().load().expect("partial config should load");

        assert_eq!(config.claude.token, "partial");
        assert_eq!(config.git.gh_username, "");
        assert_eq!(config.git.gh_token, "");
        assert_eq!(config.git.user_name, "");
        assert_eq!(config.git.user_email, "");
        assert_eq!(config.gcloud.project, "");
        assert_eq!(config.gcloud.service_account, "");
        assert_eq!(config.gcloud.service_account_key_file, "");
        assert_eq!(config.gcloud.region, DEFAULT_GCLOUD_REGION);
        assert_eq!(config.gcloud.zone, DEFAULT_GCLOUD_ZONE);
        assert_eq!(
            config.projects.get("demo"),
            Some(&ProjectConfig {
                name: "Demo".to_string(),
                path: "/tmp/demo".to_string(),
                image: None,
                remote_url: String::new(),
                target_branch: String::new(),
                env_files: Vec::new(),
                gcloud: ProjectGcloudConfig::default(),
            })
        );
    }

    #[test]
    fn project_gcloud_overrides_deserialize() {
        let temp_dir = TestDir::new();

        write_file(
            &temp_dir.config_path(),
            "[gcloud]\nproject = \"default-project\"\nregion = \"us-east4\"\n\n[projects.demo]\nname = \"Demo\"\npath = \"/tmp/demo\"\n\n[projects.demo.gcloud]\nproject = \"override-project\"\nregion = \"us-west1\"\nzone = \"us-west1-b\"\ndisk_size_gb = 120\n",
        )
        .expect("seed config should be written");

        let config = temp_dir.store().load().expect("config should load");
        let project = config
            .projects
            .get("demo")
            .expect("project override should exist");

        assert_eq!(project.gcloud.project.as_deref(), Some("override-project"));
        assert_eq!(project.gcloud.region.as_deref(), Some("us-west1"));
        assert_eq!(project.gcloud.zone.as_deref(), Some("us-west1-b"));
        assert_eq!(project.gcloud.disk_size_gb, Some(120));
        assert!(project.env_files.is_empty());
        assert_eq!(config.gcloud.machine_type, DEFAULT_GCLOUD_MACHINE_TYPE);
    }

    #[test]
    fn detect_initial_config_seeds_service_account_key_file_when_present() {
        let temp_dir = TestDir::new_without_config();
        let key_path = temp_dir
            .root
            .join(SILO_DIR_NAME)
            .join("demo-project-silo-workspaces.json");
        write_file(&key_path, "{\"type\":\"service_account\"}")
            .expect("service account key file should be written");

        let config = detect_initial_config_with(
            &temp_dir.root,
            |program, args| match (program, args.as_slice()) {
                ("gcloud", ["config", "get-value", "account"]) => {
                    Some("svc@demo-project.iam.gserviceaccount.com".to_string())
                }
                ("gcloud", ["config", "get-value", "project"]) => Some("demo-project".to_string()),
                _ => None,
            },
            |_| None,
        );

        assert_eq!(
            config.gcloud.service_account_key_file,
            key_path.to_string_lossy()
        );
    }

    #[test]
    fn invalid_toml_returns_parse_error() {
        let temp_dir = TestDir::new();
        let store = temp_dir.store();

        write_file(&temp_dir.config_path(), "[claude\n token = \"oops\"")
            .expect("invalid config should be seeded");

        let error = store.load_locked().expect_err("invalid toml should fail");

        assert!(matches!(error, ConfigError::Parse(_)));
    }

    #[test]
    fn concurrent_writes_do_not_corrupt_the_file() {
        let temp_dir = Arc::new(TestDir::new());
        let store = Arc::new(temp_dir.store());
        let expected = [
            "token-0".to_string(),
            "token-1".to_string(),
            "token-2".to_string(),
            "token-3".to_string(),
        ];

        let handles: Vec<_> = expected
            .iter()
            .cloned()
            .map(|token| {
                let store = Arc::clone(&store);
                thread::spawn(move || {
                    store
                        .write("claude.token", Value::String(token))
                        .expect("concurrent write should succeed");
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread should finish cleanly");
        }

        let persisted = store.load().expect("final config should load");
        assert!(expected.contains(&persisted.claude.token));

        let raw =
            fs::read_to_string(temp_dir.config_path()).expect("persisted file should be readable");
        toml::from_str::<SiloConfig>(&raw).expect("persisted file should remain valid toml");
    }

    #[test]
    fn initialize_defaults_if_missing_seeds_existing_credentials() {
        let temp_dir = TestDir::new_without_config();
        let store = temp_dir.store();

        store
            .initialize_defaults_if_missing_locked(|_| SiloConfig {
                gcloud: GcloudConfig {
                    account: "default-account@example.com".to_string(),
                    project: "default-project".to_string(),
                    ..GcloudConfig::default()
                },
                git: GitConfig {
                    gh_username: "octocat".to_string(),
                    gh_token: "gh-token".to_string(),
                    user_name: "Monalisa Octocat".to_string(),
                    user_email: "octocat@example.com".to_string(),
                },
                codex: CodexConfig {
                    token: "codex-token".to_string(),
                },
                claude: ClaudeConfig::default(),
                projects: IndexMap::new(),
            })
            .expect("initialization should succeed");

        let config = store.load().expect("seeded config should load");
        assert_eq!(config.gcloud.account, "default-account@example.com");
        assert_eq!(config.gcloud.project, "default-project");
        assert_eq!(config.gcloud.service_account, "");
        assert_eq!(config.git.gh_username, "octocat");
        assert_eq!(config.git.gh_token, "gh-token");
        assert_eq!(config.git.user_name, "Monalisa Octocat");
        assert_eq!(config.git.user_email, "octocat@example.com");
        assert_eq!(config.codex.token, "codex-token");
        assert_eq!(config.claude.token, "");
    }

    #[test]
    fn initialize_defaults_if_missing_preserves_existing_config() {
        let temp_dir = TestDir::new_without_config();
        let store = temp_dir.store();

        let existing = SiloConfig {
            gcloud: GcloudConfig {
                account: "existing-account@example.com".to_string(),
                project: "existing-project".to_string(),
                ..GcloudConfig::default()
            },
            git: GitConfig {
                gh_username: "existing-user".to_string(),
                gh_token: "existing-gh-token".to_string(),
                user_name: "Existing User".to_string(),
                user_email: "existing@example.com".to_string(),
            },
            codex: CodexConfig {
                token: "existing-codex-token".to_string(),
            },
            claude: ClaudeConfig {
                token: "existing-claude-token".to_string(),
            },
            projects: IndexMap::new(),
        };

        write_file(
            &temp_dir.config_path(),
            &serialize_config(&existing).expect("existing config should serialize"),
        )
        .expect("existing config should be written");

        store
            .initialize_defaults_if_missing_locked(|_| SiloConfig::default())
            .expect("initialization should not overwrite existing config");

        let loaded = store
            .load()
            .expect("existing config should remain readable");
        assert_eq!(loaded, existing);
    }

    struct TestDir {
        root: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let test_dir = Self::new_without_config();
            write_file(
                &test_dir.config_path(),
                &serialize_config(&SiloConfig::default()).expect("default config should serialize"),
            )
            .expect("default config should be written");
            test_dir
        }

        fn new_without_config() -> Self {
            let counter = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let unique = format!(
                "silo-config-test-{}-{}-{}",
                std::process::id(),
                counter,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let root = env::temp_dir().join(unique);
            fs::create_dir_all(root.join(SILO_DIR_NAME)).expect("test dir should be created");

            Self { root }
        }

        fn store(&self) -> ConfigStore {
            ConfigStore::from_home_dir(self.root.clone())
        }

        fn silo_dir(&self) -> PathBuf {
            self.root.join(SILO_DIR_NAME)
        }

        fn config_path(&self) -> PathBuf {
            self.silo_dir().join(CONFIG_FILE_NAME)
        }
    }

    fn detect_initial_config_with<C, D>(home_dir: &Path, command: C, detect_codex: D) -> SiloConfig
    where
        C: Fn(&str, Vec<&str>) -> Option<String>,
        D: Fn(&Path) -> Option<String>,
    {
        let mut gcloud = GcloudConfig::default();
        gcloud.account =
            command("gcloud", vec!["config", "get-value", "account"]).unwrap_or_default();
        gcloud.project =
            command("gcloud", vec!["config", "get-value", "project"]).unwrap_or_default();
        if gcloud.account.ends_with(".gserviceaccount.com") {
            gcloud.service_account = gcloud.account.clone();
            gcloud.service_account_key_file =
                detect_service_account_key_file(home_dir, &gcloud.project).unwrap_or_default();
        }

        SiloConfig {
            gcloud,
            git: GitConfig {
                gh_username: command("gh", vec!["api", "user", "--jq", ".login"])
                    .unwrap_or_default(),
                gh_token: command("gh", vec!["auth", "token"]).unwrap_or_default(),
                user_name: command("git", vec!["config", "--global", "user.name"])
                    .unwrap_or_default(),
                user_email: command("git", vec!["config", "--global", "user.email"])
                    .unwrap_or_default(),
            },
            codex: CodexConfig {
                token: detect_codex(home_dir).unwrap_or_default(),
            },
            claude: ClaudeConfig::default(),
            projects: IndexMap::new(),
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
