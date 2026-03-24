use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::daemon::state::PublishedState;

pub(crate) const TERMINAL_LAST_ACTIVE_METADATA_KEY: &str = "terminal-last-active";
pub(crate) const TERMINAL_LAST_WORKING_METADATA_KEY: &str = "terminal-last-working";
pub(crate) const TERMINAL_UNREAD_METADATA_KEY: &str = "terminal-unread";
pub(crate) const TERMINAL_WORKING_METADATA_KEY: &str = "terminal-working";
pub(crate) const WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY: &str = "workspace-agent-heartbeat-at";
pub(crate) const WORKSPACE_AGENT_FINGERPRINT_METADATA_KEY: &str = "workspace-agent-fingerprint";
const WORKSPACE_AGENT_FINGERPRINT_FILE: &str = "/home/silo/.silo/workspace-agent/fingerprint";

const METADATA_PUBLISH_ATTEMPTS: usize = 5;
const METADATA_PUBLISH_RETRY_BASE_DELAY: Duration = Duration::from_millis(100);

pub(crate) struct ComputeMetadataClient {
    project: String,
    zone: String,
    instance: String,
    client: Client,
}

impl ComputeMetadataClient {
    pub(crate) fn new(project: String, zone: String, instance: String) -> Self {
        Self {
            project,
            zone,
            instance,
            client: Client::builder()
                .build()
                .expect("reqwest blocking client should build"),
        }
    }

    pub(crate) fn publish(&self, published: &PublishedState) -> Result<(), String> {
        let token = self.fetch_access_token()?;
        let mut last_error = None;

        for attempt in 0..METADATA_PUBLISH_ATTEMPTS {
            match self.publish_once(&token, published) {
                Ok(()) => return Ok(()),
                Err(error)
                    if attempt + 1 < METADATA_PUBLISH_ATTEMPTS && should_retry_publish(&error) =>
                {
                    last_error = Some(error);
                    thread::sleep(METADATA_PUBLISH_RETRY_BASE_DELAY * 2u32.pow(attempt as u32));
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or_else(|| "metadata publish failed".to_string()))
    }

    fn publish_once(&self, token: &str, published: &PublishedState) -> Result<(), String> {
        let (fingerprint, items) = self.fetch_instance_metadata(token)?;
        let items = flat_metadata_items(items, published)?;

        let items = items
            .into_iter()
            .map(|(key, value)| json!({ "key": key, "value": value }))
            .collect::<Vec<_>>();
        let body = json!({
            "fingerprint": fingerprint,
            "items": items,
        });

        let url = format!(
            "https://compute.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}/setMetadata",
            self.project, self.zone, self.instance
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .map_err(|error| format!("failed to call setMetadata: {error}"))?;
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().unwrap_or_default();
        Err(format!("setMetadata failed with status {status}: {body}"))
    }

    pub(crate) fn suspend(&self) -> Result<(), String> {
        let token = self.fetch_access_token()?;
        let url = format!(
            "https://compute.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}/suspend",
            self.project, self.zone, self.instance
        );
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .send()
            .map_err(|error| format!("failed to call suspend: {error}"))?;
        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().unwrap_or_default();
        Err(format!("suspend failed with status {status}: {body}"))
    }

    fn fetch_access_token(&self) -> Result<String, String> {
        let response = self
            .client
            .get("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token")
            .header("Metadata-Flavor", "Google")
            .send()
            .map_err(|error| format!("failed to get metadata access token: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(format!(
                "metadata access token request failed with status {status}: {body}"
            ));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        response
            .json::<TokenResponse>()
            .map(|response| response.access_token)
            .map_err(|error| format!("failed to parse metadata access token response: {error}"))
    }

    fn fetch_instance_metadata(
        &self,
        token: &str,
    ) -> Result<(String, BTreeMap<String, String>), String> {
        let url = format!(
            "https://compute.googleapis.com/compute/v1/projects/{}/zones/{}/instances/{}",
            self.project, self.zone, self.instance
        );
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|error| format!("failed to fetch instance metadata: {error}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().unwrap_or_default();
            return Err(format!(
                "instance metadata fetch failed with status {status}: {body}"
            ));
        }

        let value = response
            .json::<Value>()
            .map_err(|error| format!("failed to parse instance metadata response: {error}"))?;
        let fingerprint = value
            .get("metadata")
            .and_then(|metadata| metadata.get("fingerprint"))
            .and_then(Value::as_str)
            .ok_or_else(|| "instance metadata response is missing fingerprint".to_string())?
            .to_string();
        let items = value
            .get("metadata")
            .and_then(|metadata| metadata.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut map = BTreeMap::new();
        for item in items {
            let Some(key) = item.get("key").and_then(Value::as_str) else {
                continue;
            };
            let Some(value) = item.get("value").and_then(Value::as_str) else {
                continue;
            };
            map.insert(key.to_string(), value.to_string());
        }

        Ok((fingerprint, map))
    }
}

pub(crate) fn flat_metadata_items(
    mut items: BTreeMap<String, String>,
    published: &PublishedState,
) -> Result<BTreeMap<String, String>, String> {
    update_metadata_item(&mut items, "branch", published.branch.as_deref());
    update_metadata_item(
        &mut items,
        TERMINAL_UNREAD_METADATA_KEY,
        Some(bool_metadata_value(published.unread)),
    );
    update_metadata_item(
        &mut items,
        TERMINAL_WORKING_METADATA_KEY,
        Some(bool_metadata_value(published.working)),
    );
    update_metadata_item(
        &mut items,
        TERMINAL_LAST_ACTIVE_METADATA_KEY,
        published.last_active.as_deref(),
    );
    update_metadata_item(
        &mut items,
        TERMINAL_LAST_WORKING_METADATA_KEY,
        published.last_working.as_deref(),
    );
    update_metadata_item(
        &mut items,
        WORKSPACE_AGENT_HEARTBEAT_METADATA_KEY,
        Some(published.heartbeat_at.as_str()),
    );
    update_metadata_item(
        &mut items,
        WORKSPACE_AGENT_FINGERPRINT_METADATA_KEY,
        current_agent_fingerprint().as_deref(),
    );
    Ok(items)
}

fn current_agent_fingerprint() -> Option<String> {
    fs::read_to_string(WORKSPACE_AGENT_FINGERPRINT_FILE)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn update_metadata_item(
    items: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&str>,
) {
    match value.map(str::trim) {
        Some(value) if !value.is_empty() => {
            items.insert(key.to_string(), value.to_string());
        }
        _ => {
            items.remove(key);
        }
    }
}

pub(crate) fn bool_metadata_value(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn should_retry_publish(error: &str) -> bool {
    error.contains("status 412")
        || error.contains("conditionNotMet")
        || error.contains("status 403")
            && (error.contains("Too many pending operations on a resource.")
                || error.contains("rateLimitExceeded"))
}
