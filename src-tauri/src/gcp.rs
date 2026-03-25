use crate::config::ConfigStore;
use crate::state_paths;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, Method, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::async_runtime;
use tokio::sync::Mutex as AsyncMutex;

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const COMPUTE_API_BASE: &str = "https://compute.googleapis.com/compute/v1";
const OSLOGIN_API_BASE: &str = "https://oslogin.googleapis.com/v1";
const OSLOGIN_API_BASE_BETA: &str = "https://oslogin.googleapis.com/v1beta";
const SSH_KEY_TTL_SECS: i64 = 600;
const ACCESS_TOKEN_REFRESH_MARGIN_SECS: i64 = 60;
const OSLOGIN_IMPORT_RETRY_ATTEMPTS: usize = 4;
const OSLOGIN_IMPORT_RETRY_INITIAL_DELAY: Duration = Duration::from_millis(250);
const OSLOGIN_KEY_REFRESH_MARGIN: Duration = Duration::from_secs(60);
const OSLOGIN_PROPAGATION_WAIT: Duration = Duration::from_secs(5);
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

static CLIENT: OnceLock<Client> = OnceLock::new();
static ACCESS_TOKENS: OnceLock<Mutex<HashMap<String, CachedAccessToken>>> = OnceLock::new();
static OSLOGIN_USERNAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static OSLOGIN_GATES: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
static OSLOGIN_SSH_KEYS: OnceLock<Mutex<HashMap<String, CachedOsLoginSshKey>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub(crate) struct InstanceEndpoint {
    pub(crate) host: String,
    pub(crate) zone: String,
}

#[derive(Debug, Clone)]
pub(crate) struct OsLoginSession {
    pub(crate) username: String,
    pub(crate) key_path: PathBuf,
}

#[derive(Debug, Clone)]
struct CachedAccessToken {
    value: String,
    refresh_at: Instant,
}

#[derive(Debug, Clone)]
struct CachedOsLoginSshKey {
    username: String,
    key_path: PathBuf,
    import_valid_until: Instant,
}

#[derive(Debug, Clone, Deserialize)]
struct ServiceAccountCredentials {
    client_email: String,
    private_key: String,
    #[serde(default)]
    token_uri: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServiceAccountClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: usize,
    iat: usize,
}

fn client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            .build()
            .expect("reqwest client should build")
    })
}

fn access_tokens() -> &'static Mutex<HashMap<String, CachedAccessToken>> {
    ACCESS_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn oslogin_usernames() -> &'static Mutex<HashMap<String, String>> {
    OSLOGIN_USERNAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn oslogin_gates() -> &'static Mutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    OSLOGIN_GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn oslogin_ssh_keys() -> &'static Mutex<HashMap<String, CachedOsLoginSshKey>> {
    OSLOGIN_SSH_KEYS.get_or_init(|| Mutex::new(HashMap::new()))
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

fn runtime_identity() -> Result<(String, String), String> {
    let config = ConfigStore::new()
        .and_then(|store| store.load())
        .map_err(|error| error.to_string())?;
    let service_account = config.gcloud.service_account.trim().to_string();
    let key_file = config.gcloud.service_account_key_file.trim().to_string();
    if service_account.is_empty() || key_file.is_empty() {
        return Err("service account credentials are not configured".to_string());
    }
    Ok((service_account, key_file))
}

pub(crate) fn runtime_identity_configured() -> bool {
    let Ok((_service_account, key_file)) = runtime_identity() else {
        return false;
    };
    Path::new(&key_file).is_file()
}

async fn access_token() -> Result<String, String> {
    let (_service_account, key_file) = runtime_identity()?;
    if let Some(cached) = access_tokens()
        .lock()
        .map_err(|_| "access token cache lock poisoned".to_string())?
        .get(&key_file)
        .cloned()
        .filter(|cached| Instant::now() < cached.refresh_at)
    {
        return Ok(cached.value);
    }

    let contents = async_runtime::spawn_blocking({
        let key_file = key_file.clone();
        move || fs::read_to_string(&key_file)
    })
    .await
    .map_err(|error| format!("credentials read task failed: {error}"))?
    .map_err(|error| format!("failed to read service account credentials: {error}"))?;
    let credentials: ServiceAccountCredentials = serde_json::from_str(&contents)
        .map_err(|error| format!("invalid service account credentials json: {error}"))?;
    let token_uri = credentials
        .token_uri
        .clone()
        .unwrap_or_else(|| DEFAULT_TOKEN_URI.to_string());
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let claims = ServiceAccountClaims {
        iss: &credentials.client_email,
        scope: CLOUD_PLATFORM_SCOPE,
        aud: &token_uri,
        iat: now as usize,
        exp: (now + SSH_KEY_TTL_SECS) as usize,
    };
    let assertion = jsonwebtoken::encode(
        &Header::new(Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(credentials.private_key.as_bytes())
            .map_err(|error| format!("failed to parse service account private key: {error}"))?,
    )
    .map_err(|error| format!("failed to sign service account jwt: {error}"))?;
    let response = client()
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("token request failed: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("token request failed with status {status}: {body}"));
    }
    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        expires_in: i64,
    }
    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|error| format!("invalid token response: {error}"))?;
    let bearer = format!("Bearer {}", token.access_token);
    let refresh_at = Instant::now()
        + Duration::from_secs(
            (token.expires_in - ACCESS_TOKEN_REFRESH_MARGIN_SECS)
                .max(30)
                .try_into()
                .unwrap_or(30),
        );
    access_tokens()
        .lock()
        .map_err(|_| "access token cache lock poisoned".to_string())?
        .insert(
            key_file,
            CachedAccessToken {
                value: bearer.clone(),
                refresh_at,
            },
        );
    Ok(bearer)
}

async fn authorized(builder: RequestBuilder) -> Result<RequestBuilder, String> {
    let token = access_token().await?;
    Ok(builder.header("Authorization", token))
}

async fn send_json(builder: RequestBuilder, context: &str) -> Result<Value, String> {
    let response = builder
        .send()
        .await
        .map_err(|error| format!("{context}: request failed: {error}"))?;
    if response.status().is_success() {
        return response
            .json::<Value>()
            .await
            .map_err(|error| format!("{context}: invalid response json: {error}"));
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("{context}: status {status}: {body}"))
}

async fn send_empty(builder: RequestBuilder, context: &str) -> Result<(), String> {
    let response = builder
        .send()
        .await
        .map_err(|error| format!("{context}: request failed: {error}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("{context}: status {status}: {body}"))
}

fn metadata_items_to_map(value: Option<&Value>) -> Map<String, Value> {
    let mut map = Map::new();
    let items = value
        .and_then(|metadata| metadata.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        let Some(key) = item.get("key").and_then(Value::as_str) else {
            continue;
        };
        let Some(value) = item.get("value").and_then(Value::as_str) else {
            continue;
        };
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    map
}

fn oslogin_username(profile: &Value) -> Option<String> {
    profile
        .get("posixAccounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|account| account.get("username").and_then(Value::as_str))
        .map(str::trim)
        .find(|username| !username.is_empty())
        .map(str::to_owned)
}

fn response_login_profile_username(response: &Value) -> Option<String> {
    response.get("loginProfile").and_then(oslogin_username)
}

fn posix_account_username(account: &Value) -> Option<String> {
    account
        .get("username")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|username| !username.is_empty())
        .map(str::to_owned)
}

fn oslogin_username_cache_key(project: &str, account: &str) -> String {
    format!("{project}:{account}")
}

fn oslogin_ssh_key_cache_key(project: &str, zone: &str, workspace: &str, account: &str) -> String {
    format!("{project}:{zone}:{workspace}:{account}")
}

fn oslogin_gate(project: &str, account: &str) -> Result<Arc<AsyncMutex<()>>, String> {
    let key = oslogin_username_cache_key(project, account);
    let mut guard = oslogin_gates()
        .lock()
        .map_err(|_| "OS Login gate cache lock poisoned".to_string())?;
    Ok(guard
        .entry(key)
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone())
}

fn cached_oslogin_username(project: &str, account: &str) -> Result<Option<String>, String> {
    Ok(oslogin_usernames()
        .lock()
        .map_err(|_| "OS Login username cache lock poisoned".to_string())?
        .get(&oslogin_username_cache_key(project, account))
        .cloned())
}

fn store_oslogin_username(project: &str, account: &str, username: &str) -> Result<(), String> {
    oslogin_usernames()
        .lock()
        .map_err(|_| "OS Login username cache lock poisoned".to_string())?
        .insert(
            oslogin_username_cache_key(project, account),
            username.to_string(),
        );
    Ok(())
}

fn cached_oslogin_ssh_key(
    project: &str,
    zone: &str,
    workspace: &str,
    account: &str,
) -> Result<Option<OsLoginSession>, String> {
    let cache_key = oslogin_ssh_key_cache_key(project, zone, workspace, account);
    let mut guard = oslogin_ssh_keys()
        .lock()
        .map_err(|_| "OS Login SSH key cache lock poisoned".to_string())?;
    let Some(cached) = guard.get(&cache_key).cloned() else {
        return Ok(None);
    };
    if Instant::now() + OSLOGIN_KEY_REFRESH_MARGIN >= cached.import_valid_until
        || !cached.key_path.exists()
    {
        guard.remove(&cache_key);
        return Ok(None);
    }

    Ok(Some(OsLoginSession {
        username: cached.username,
        key_path: cached.key_path,
    }))
}

fn store_oslogin_ssh_key(
    project: &str,
    zone: &str,
    workspace: &str,
    account: &str,
    username: &str,
    key_path: &Path,
    import_valid_until: Instant,
) -> Result<(), String> {
    oslogin_ssh_keys()
        .lock()
        .map_err(|_| "OS Login SSH key cache lock poisoned".to_string())?
        .insert(
            oslogin_ssh_key_cache_key(project, zone, workspace, account),
            CachedOsLoginSshKey {
                username: username.to_string(),
                key_path: key_path.to_path_buf(),
                import_valid_until,
            },
        );
    Ok(())
}

#[cfg(test)]
fn clear_oslogin_ssh_key(
    project: &str,
    zone: &str,
    workspace: &str,
    account: &str,
) -> Result<(), String> {
    oslogin_ssh_keys()
        .lock()
        .map_err(|_| "OS Login SSH key cache lock poisoned".to_string())?
        .remove(&oslogin_ssh_key_cache_key(
            project, zone, workspace, account,
        ));
    Ok(())
}

fn is_already_exists_error(error: &str) -> bool {
    error.contains("status 409") || error.contains("ALREADY_EXISTS")
}

fn is_retryable_oslogin_import_error(error: &str) -> bool {
    error.contains("ABORTED") || error.contains("Multiple concurrent mutations were attempted")
}

fn oslogin_import_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(OSLOGIN_IMPORT_RETRY_INITIAL_DELAY.as_millis() as u64 * (1u64 << attempt))
}

fn metadata_map_to_items(map: &Map<String, Value>) -> Vec<Value> {
    map.iter()
        .filter_map(|(key, value)| {
            value.as_str().map(|value| {
                json!({
                    "key": key,
                    "value": value,
                })
            })
        })
        .collect()
}

fn network_host(instance: &Value, project: &str) -> Option<String> {
    instance
        .get("networkInterfaces")
        .and_then(Value::as_array)
        .and_then(|interfaces| interfaces.first())
        .and_then(|iface| iface.get("accessConfigs"))
        .and_then(Value::as_array)
        .and_then(|configs| configs.first())
        .and_then(|config| config.get("natIP"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let name = instance.get("name").and_then(Value::as_str)?;
            let zone = instance
                .get("zone")
                .and_then(Value::as_str)
                .and_then(|value| value.rsplit('/').next())?;
            Some(format!("{name}.{zone}.c.{project}.internal"))
        })
}

fn runtime_ssh_dir() -> Result<PathBuf, String> {
    let dir = state_paths::app_state_dir()?.join("ssh");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

fn runtime_ssh_control_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("silo-ssh");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

fn ssh_slug(project: &str, zone: &str, workspace: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project.as_bytes());
    hasher.update([0]);
    hasher.update(zone.as_bytes());
    hasher.update([0]);
    hasher.update(workspace.as_bytes());
    let digest = hasher.finalize();
    let encoded = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    encoded[..20].to_string()
}

pub(crate) fn ssh_known_hosts_path() -> Result<PathBuf, String> {
    Ok(runtime_ssh_dir()?.join("known_hosts"))
}

pub(crate) fn ssh_control_path(
    project: &str,
    zone: &str,
    workspace: &str,
) -> Result<PathBuf, String> {
    Ok(runtime_ssh_control_dir()?.join(format!("cm-{}", ssh_slug(project, zone, workspace))))
}

pub(crate) fn ssh_key_path(project: &str, zone: &str, workspace: &str) -> Result<PathBuf, String> {
    Ok(runtime_ssh_dir()?.join(format!("key-{}", ssh_slug(project, zone, workspace))))
}

fn ssh_public_key_path(key_path: &Path) -> Result<PathBuf, String> {
    let filename = key_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid ssh key path: {}", key_path.display()))?;
    Ok(key_path.with_file_name(format!("{filename}.pub")))
}

pub(crate) async fn list_instances(project: &str) -> Result<Vec<Value>, String> {
    let mut page_token: Option<String> = None;
    let mut instances = Vec::new();
    loop {
        let url = format!("{COMPUTE_API_BASE}/projects/{project}/aggregated/instances");
        let mut request = authorized(client().request(Method::GET, &url))
            .await?
            .query(&[(
                "fields",
                "items/*/instances(name,zone,status,labels,metadata,creationTimestamp,networkInterfaces,disks),nextPageToken",
            )]);
        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }
        let response = send_json(request, "failed to list workspaces").await?;
        if let Some(items) = response.get("items").and_then(Value::as_object) {
            for entry in items.values() {
                let zone_instances = entry
                    .get("instances")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                instances.extend(zone_instances);
            }
        }
        page_token = response
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if page_token.is_none() {
            return Ok(instances);
        }
    }
}

pub(crate) async fn get_instance(project: &str, zone: &str, name: &str) -> Result<Value, String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/zones/{zone}/instances/{name}");
    let request = authorized(client().request(Method::GET, &url))
        .await?
        .query(&[(
            "fields",
            "name,zone,status,labels,metadata,creationTimestamp,networkInterfaces,disks",
        )]);
    send_json(request, "failed to describe workspace").await
}

pub(crate) async fn instance_endpoint(
    project: &str,
    zone: &str,
    name: &str,
) -> Result<InstanceEndpoint, String> {
    let instance = get_instance(project, zone, name).await?;
    let host = network_host(&instance, project)
        .ok_or_else(|| format!("workspace {name} does not expose a reachable host"))?;
    Ok(InstanceEndpoint {
        host,
        zone: zone.to_string(),
    })
}

pub(crate) async fn post_instance_action(
    project: &str,
    zone: &str,
    name: &str,
    action: &str,
    context: &str,
) -> Result<(), String> {
    let url =
        format!("{COMPUTE_API_BASE}/projects/{project}/zones/{zone}/instances/{name}/{action}");
    let request = authorized(client().request(Method::POST, &url)).await?;
    send_empty(request, context).await
}

pub(crate) async fn delete_instance(project: &str, zone: &str, name: &str) -> Result<(), String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/zones/{zone}/instances/{name}");
    let request = authorized(client().request(Method::DELETE, &url)).await?;
    send_empty(request, "failed to delete workspace").await
}

pub(crate) async fn set_instance_metadata(
    project: &str,
    zone: &str,
    name: &str,
    updates: &[(&str, Option<&str>)],
) -> Result<(), String> {
    let instance = get_instance(project, zone, name).await?;
    let metadata = instance
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| json!({ "items": [] }));
    let fingerprint = metadata
        .get("fingerprint")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("workspace {name} metadata is missing fingerprint"))?;
    let mut items = metadata_items_to_map(Some(&metadata));
    for (key, value) in updates {
        match value.map(str::trim) {
            Some(value) if !value.is_empty() => {
                items.insert((*key).to_string(), Value::String(value.to_string()));
            }
            _ => {
                items.remove(*key);
            }
        }
    }
    let body = json!({
        "fingerprint": fingerprint,
        "items": metadata_map_to_items(&items),
    });
    let url =
        format!("{COMPUTE_API_BASE}/projects/{project}/zones/{zone}/instances/{name}/setMetadata");
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .json(&body);
    send_empty(
        request,
        &format!("failed to update metadata for workspace {name}"),
    )
    .await
}

pub(crate) async fn get_image_from_family(project: &str, family: &str) -> Result<String, String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/images/family/{family}");
    let request = authorized(client().request(Method::GET, &url))
        .await?
        .query(&[("fields", "selfLink")]);
    let response = send_json(request, "failed to resolve image family").await?;
    response
        .get("selfLink")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "image family response is missing selfLink".to_string())
}

pub(crate) async fn create_instance(project: &str, zone: &str, body: Value) -> Result<(), String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/zones/{zone}/instances");
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .json(&body);
    send_empty(request, "failed to create workspace").await
}

pub(crate) async fn list_snapshots(project: &str) -> Result<Vec<Value>, String> {
    let mut page_token: Option<String> = None;
    let mut snapshots = Vec::new();
    loop {
        let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/snapshots");
        let mut request = authorized(client().request(Method::GET, &url))
            .await?
            .query(&[(
                "fields",
                "items(name,status,creationTimestamp,labels,labelFingerprint),nextPageToken",
            )]);
        if let Some(token) = page_token.as_deref() {
            request = request.query(&[("pageToken", token)]);
        }
        let response = send_json(request, "failed to list template snapshots").await?;
        let page_items = response
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        snapshots.extend(page_items);
        page_token = response
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if page_token.is_none() {
            return Ok(snapshots);
        }
    }
}

pub(crate) async fn get_snapshot(project: &str, name: &str) -> Result<Value, String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/snapshots/{name}");
    let request = authorized(client().request(Method::GET, &url))
        .await?
        .query(&[(
            "fields",
            "name,status,creationTimestamp,labels,labelFingerprint",
        )]);
    send_json(request, "failed to describe template snapshot").await
}

pub(crate) async fn delete_snapshot(project: &str, name: &str) -> Result<(), String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/snapshots/{name}");
    let request = authorized(client().request(Method::DELETE, &url)).await?;
    send_empty(request, "failed to delete template snapshot").await
}

pub(crate) async fn create_snapshot(project: &str, body: Value) -> Result<(), String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/snapshots");
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .json(&body);
    send_empty(request, "failed to create template snapshot").await
}

pub(crate) async fn set_snapshot_labels(
    project: &str,
    name: &str,
    labels: Map<String, Value>,
    fingerprint: &str,
) -> Result<(), String> {
    let url = format!("{COMPUTE_API_BASE}/projects/{project}/global/snapshots/{name}/setLabels");
    let body = json!({
        "labels": labels,
        "labelFingerprint": fingerprint,
    });
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .json(&body);
    send_empty(request, "failed to update template snapshot labels").await
}

pub(crate) async fn import_oslogin_key(
    project: &str,
    account: &str,
    public_key: &str,
) -> Result<Option<String>, String> {
    let expires_at = time::OffsetDateTime::now_utc() + time::Duration::seconds(SSH_KEY_TTL_SECS);
    let url = format!("{OSLOGIN_API_BASE}/users/{account}:importSshPublicKey");
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .query(&[("projectId", project)])
        .json(&json!({
            "key": public_key,
            "expirationTimeUsec": expires_at.unix_timestamp_nanos() / 1_000,
        }));
    let response = send_json(request, "failed to import OS Login SSH public key").await?;
    let username = response_login_profile_username(&response);
    if let Some(username) = username.as_deref() {
        store_oslogin_username(project, account, username)?;
    }
    Ok(username)
}

pub(crate) async fn get_login_profile_username(
    project: &str,
    account: &str,
) -> Result<String, String> {
    if let Some(username) = cached_oslogin_username(project, account)? {
        return Ok(username);
    }
    let url = format!("{OSLOGIN_API_BASE}/users/{account}/loginProfile");
    let request = authorized(client().request(Method::GET, &url))
        .await?
        .query(&[("projectId", project)]);
    let response = send_json(request, "failed to load OS Login profile").await?;
    let username = oslogin_username(&response)
        .ok_or_else(|| "OS Login profile is missing a POSIX username".to_string())?;
    store_oslogin_username(project, account, &username)?;
    Ok(username)
}

pub(crate) async fn provision_oslogin_posix_account(
    project: &str,
    account: &str,
) -> Result<String, String> {
    if let Some(username) = cached_oslogin_username(project, account)? {
        return Ok(username);
    }
    let url = format!("{OSLOGIN_API_BASE_BETA}/users/{account}/projects/{project}");
    let request = authorized(client().request(Method::POST, &url))
        .await?
        .json(&json!({}));
    match send_json(request, "failed to provision OS Login POSIX account").await {
        Ok(response) => {
            let username = posix_account_username(&response).ok_or_else(|| {
                "OS Login POSIX account response is missing a username".to_string()
            })?;
            store_oslogin_username(project, account, &username)?;
            Ok(username)
        }
        Err(error) if is_already_exists_error(&error) => {
            get_login_profile_username(project, account).await
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn ensure_runtime_oslogin_ready(project: &str) -> Result<String, String> {
    let (service_account, _key_file) = runtime_identity()?;
    match get_login_profile_username(project, &service_account).await {
        Ok(username) => Ok(username),
        Err(error) if error.contains("OS Login profile is missing a POSIX username") => {
            provision_oslogin_posix_account(project, &service_account).await
        }
        Err(error) => Err(error),
    }
}

async fn sleep_for(duration: Duration, context: &str) -> Result<(), String> {
    async_runtime::spawn_blocking(move || std::thread::sleep(duration))
        .await
        .map_err(|error| format!("{context}: {error}"))?;
    Ok(())
}

async fn ensure_workspace_ssh_keypair(key_path: &Path) -> Result<PathBuf, String> {
    let public_key_path = ssh_public_key_path(key_path)?;
    if key_path.exists() && public_key_path.exists() {
        return Ok(public_key_path);
    }
    if key_path.exists() {
        let _ = fs::remove_file(key_path);
    }
    if public_key_path.exists() {
        let _ = fs::remove_file(&public_key_path);
    }

    let key_path_string = key_path.to_string_lossy().into_owned();
    async_runtime::spawn_blocking(move || {
        let status = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f", &key_path_string])
            .status()
            .map_err(|error| format!("failed to start ssh-keygen: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ssh-keygen exited with status {status}"))
        }
    })
    .await
    .map_err(|error| format!("ssh-keygen task failed: {error}"))??;

    Ok(public_key_path)
}

async fn read_public_key(public_key_path: &Path) -> Result<String, String> {
    let public_key_path = public_key_path.to_path_buf();
    async_runtime::spawn_blocking(move || fs::read_to_string(&public_key_path))
        .await
        .map_err(|error| format!("public key read task failed: {error}"))?
        .map_err(|error| format!("failed to read generated public key: {error}"))
}

async fn import_oslogin_key_with_retry(
    project: &str,
    account: &str,
    public_key: &str,
) -> Result<(Option<String>, usize), String> {
    for attempt in 0..OSLOGIN_IMPORT_RETRY_ATTEMPTS {
        match import_oslogin_key(project, account, public_key).await {
            Ok(username) => return Ok((username, attempt)),
            Err(error)
                if attempt + 1 < OSLOGIN_IMPORT_RETRY_ATTEMPTS
                    && is_retryable_oslogin_import_error(&error) =>
            {
                let delay = oslogin_import_retry_delay(attempt);
                log::warn!(
                    "retrying OS Login SSH key import project={} account={} attempt={}/{} delay_ms={} error={}",
                    project,
                    account,
                    attempt + 1,
                    OSLOGIN_IMPORT_RETRY_ATTEMPTS,
                    delay.as_millis(),
                    error
                );
                sleep_for(delay, "OS Login import retry wait failed").await?;
            }
            Err(error) => return Err(error),
        }
    }

    Err("OS Login SSH key import retry loop exhausted".to_string())
}

pub(crate) async fn prepare_oslogin_session(
    project: &str,
    zone: &str,
    workspace: &str,
) -> Result<OsLoginSession, String> {
    let (service_account, _key_file) = runtime_identity()?;
    let key_path = ssh_key_path(project, zone, workspace)?;
    let parent = key_path
        .parent()
        .ok_or_else(|| format!("invalid ssh key path: {}", key_path.display()))?;
    ensure_private_dir(parent)?;
    if let Some(session) = cached_oslogin_ssh_key(project, zone, workspace, &service_account)? {
        log::debug!(
            "reusing cached OS Login SSH key project={} zone={} workspace={}",
            project,
            zone,
            workspace
        );
        return Ok(session);
    }

    let public_key_path = ensure_workspace_ssh_keypair(&key_path).await?;
    let public_key = read_public_key(&public_key_path).await?;
    let gate = oslogin_gate(project, &service_account)?;
    let _guard = gate.lock().await;
    if let Some(session) = cached_oslogin_ssh_key(project, zone, workspace, &service_account)? {
        log::debug!(
            "reusing cached OS Login SSH key after gate project={} zone={} workspace={}",
            project,
            zone,
            workspace
        );
        return Ok(session);
    }

    log::info!(
        "refreshing OS Login SSH key lease project={} zone={} workspace={}",
        project,
        zone,
        workspace
    );
    let (imported_username, retries) =
        import_oslogin_key_with_retry(project, &service_account, public_key.trim()).await?;
    let username = match imported_username {
        Some(username) => username,
        None => provision_oslogin_posix_account(project, &service_account).await?,
    };
    if retries > 0 {
        log::info!(
            "OS Login SSH key import succeeded after retries project={} zone={} workspace={} retries={}",
            project,
            zone,
            workspace,
            retries
        );
    }
    sleep_for(OSLOGIN_PROPAGATION_WAIT, "OS Login propagation wait failed").await?;
    store_oslogin_ssh_key(
        project,
        zone,
        workspace,
        &service_account,
        &username,
        &key_path,
        Instant::now() + Duration::from_secs(SSH_KEY_TTL_SECS as u64),
    )?;

    Ok(OsLoginSession { username, key_path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oslogin_username_skips_empty_accounts() {
        let value = json!({
            "posixAccounts": [
                { "username": "" },
                { "uid": "12345" },
                { "username": "svc_user" }
            ]
        });

        assert_eq!(oslogin_username(&value).as_deref(), Some("svc_user"));
    }

    #[test]
    fn response_login_profile_username_reads_nested_profile() {
        let value = json!({
            "loginProfile": {
                "posixAccounts": [
                    { "uid": "12345" },
                    { "username": "svc_user" }
                ]
            }
        });

        assert_eq!(
            response_login_profile_username(&value).as_deref(),
            Some("svc_user")
        );
    }

    #[test]
    fn already_exists_error_recognizes_common_api_variants() {
        assert!(is_already_exists_error("status 409 Conflict"));
        assert!(is_already_exists_error("reason: ALREADY_EXISTS"));
        assert!(!is_already_exists_error("status 403 Forbidden"));
    }

    #[test]
    fn retryable_oslogin_import_error_matches_aborted_conflicts() {
        assert!(is_retryable_oslogin_import_error(
            "failed to import OS Login SSH public key: status 409 Conflict: ABORTED"
        ));
        assert!(is_retryable_oslogin_import_error(
            "Multiple concurrent mutations were attempted. Please retry the request."
        ));
        assert!(!is_retryable_oslogin_import_error(
            "failed to import OS Login SSH public key: status 403 Forbidden"
        ));
    }

    #[test]
    fn oslogin_import_retry_delay_exponential_backoff() {
        assert_eq!(oslogin_import_retry_delay(0), Duration::from_millis(250));
        assert_eq!(oslogin_import_retry_delay(1), Duration::from_millis(500));
        assert_eq!(oslogin_import_retry_delay(2), Duration::from_millis(1000));
    }

    #[test]
    fn cached_oslogin_ssh_key_reuses_valid_lease() {
        let unique = format!(
            "silo-gcp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let key_path = std::env::temp_dir().join(format!("{unique}.key"));
        fs::write(&key_path, "test-key").unwrap();

        store_oslogin_ssh_key(
            "project",
            "zone",
            &unique,
            "account",
            "svc_user",
            &key_path,
            Instant::now() + OSLOGIN_KEY_REFRESH_MARGIN + Duration::from_secs(5),
        )
        .unwrap();

        let session = cached_oslogin_ssh_key("project", "zone", &unique, "account")
            .unwrap()
            .expect("lease should be reusable");
        assert_eq!(session.username, "svc_user");
        assert_eq!(session.key_path, key_path);

        clear_oslogin_ssh_key("project", "zone", &unique, "account").unwrap();
        let _ = fs::remove_file(&key_path);
    }

    #[test]
    fn cached_oslogin_ssh_key_expires_with_refresh_margin() {
        let unique = format!(
            "silo-gcp-expiring-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let key_path = std::env::temp_dir().join(format!("{unique}.key"));
        fs::write(&key_path, "test-key").unwrap();

        store_oslogin_ssh_key(
            "project",
            "zone",
            &unique,
            "account",
            "svc_user",
            &key_path,
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();

        assert!(
            cached_oslogin_ssh_key("project", "zone", &unique, "account")
                .unwrap()
                .is_none()
        );

        let _ = fs::remove_file(&key_path);
    }

    #[test]
    fn cached_oslogin_ssh_key_drops_missing_key_path() {
        let unique = format!(
            "silo-gcp-missing-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let key_path = std::env::temp_dir().join(format!("{unique}.key"));

        store_oslogin_ssh_key(
            "project",
            "zone",
            &unique,
            "account",
            "svc_user",
            &key_path,
            Instant::now() + OSLOGIN_KEY_REFRESH_MARGIN + Duration::from_secs(5),
        )
        .unwrap();

        assert!(
            cached_oslogin_ssh_key("project", "zone", &unique, "account")
                .unwrap()
                .is_none()
        );
    }
}
