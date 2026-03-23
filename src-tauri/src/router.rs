use crate::remote::spawn_remote_port_forward;
use crate::tls;
use crate::workspaces::{WorkspaceLookup, WorkspaceSession};
use std::collections::{HashMap, HashSet};
use std::net::{TcpListener, TcpStream};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::webview::Url;

const ROUTER_ATTACH_TIMEOUT: Duration = Duration::from_secs(10);
const ROUTER_ATTACH_RETRY_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Default)]
pub(crate) struct RouterManager {
    routes: Arc<Mutex<HashMap<String, HashMap<u16, LoopbackRoute>>>>,
}

struct LoopbackRoute {
    local_port: u16,
    child: Child,
}

impl RouterManager {
    pub(crate) fn ensure_loopback_route(
        &self,
        lookup: &WorkspaceLookup,
        scheme: &str,
        remote_port: u16,
    ) -> Result<u16, String> {
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| "browser router lock poisoned".to_string())?;
        let workspace_routes = routes
            .entry(lookup.workspace.name().to_string())
            .or_insert_with(HashMap::new);

        if let Some(route) = workspace_routes.get_mut(&remote_port) {
            if route
                .child
                .try_wait()
                .map_err(|error| format!("failed to inspect loopback route: {error}"))?
                .is_none()
            {
                return Ok(route.local_port);
            }

            let _ = route.child.wait();
            workspace_routes.remove(&remote_port);
        }

        let local_port = find_free_local_port()?;
        let mut child = spawn_loopback_route(lookup, local_port, remote_port)?;
        wait_for_local_route(local_port, &mut child)?;
        workspace_routes.insert(remote_port, LoopbackRoute { local_port, child });
        log::info!(
            "created {} workspace loopback route workspace={} remote_port={} local_port={}",
            scheme,
            lookup.workspace.name(),
            remote_port,
            local_port
        );
        Ok(local_port)
    }

    pub(crate) fn rewrite_loopback_url(
        &self,
        lookup: &WorkspaceLookup,
        logical_url: &str,
    ) -> Result<Option<String>, String> {
        let mut parsed =
            Url::parse(logical_url).map_err(|error| format!("invalid browser url: {error}"))?;
        let Some(host) = parsed.host_str() else {
            return Ok(None);
        };
        if !is_loopback_host(host) {
            return Ok(None);
        }

        let scheme = parsed.scheme().to_string();
        if scheme != "http" && scheme != "https" {
            return Ok(None);
        }

        let remote_port = parsed
            .port()
            .unwrap_or_else(|| tls::default_port_for_scheme(&scheme));
        let local_port = self.ensure_loopback_route(lookup, &scheme, remote_port)?;
        parsed
            .set_host(Some("localhost"))
            .map_err(|_| "failed to rewrite browser localhost route".to_string())?;
        parsed
            .set_port(Some(local_port))
            .map_err(|_| "failed to rewrite browser localhost port".to_string())?;
        Ok(Some(parsed.to_string()))
    }

    pub(crate) fn logical_url_for_reported_url(
        &self,
        workspace: &str,
        reported_url: &str,
    ) -> Option<String> {
        let mut parsed = Url::parse(reported_url).ok()?;
        let host = parsed.host_str()?;
        if !is_loopback_host(host) {
            return None;
        }

        let local_port = parsed.port()?;
        let routes = self.routes.lock().ok()?;
        let workspace_routes = routes.get(workspace)?;
        let (&remote_port, _) = workspace_routes
            .iter()
            .find(|(_, route)| route.local_port == local_port)?;
        parsed.set_port(Some(remote_port)).ok()?;
        Some(parsed.to_string())
    }

    pub(crate) fn release_unused_workspace_routes(
        &self,
        workspace: &str,
        browsers: &[WorkspaceSession],
    ) -> Result<(), String> {
        let required_ports = browsers
            .iter()
            .filter_map(|session| {
                session
                    .logical_url
                    .as_deref()
                    .or(session.url.as_deref())
                    .and_then(required_loopback_port)
            })
            .collect::<HashSet<_>>();

        let mut routes = self
            .routes
            .lock()
            .map_err(|_| "browser router lock poisoned".to_string())?;
        let Some(workspace_routes) = routes.get_mut(workspace) else {
            return Ok(());
        };

        let stale_ports = workspace_routes
            .keys()
            .copied()
            .filter(|port| !required_ports.contains(port))
            .collect::<Vec<_>>();

        for remote_port in stale_ports {
            if let Some(mut route) = workspace_routes.remove(&remote_port) {
                let _ = route.child.kill();
                let _ = route.child.wait();
            }
        }

        if workspace_routes.is_empty() {
            routes.remove(workspace);
        }
        Ok(())
    }
}

fn required_loopback_port(url: &str) -> Option<u16> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    if !is_loopback_host(host) {
        return None;
    }
    Some(
        parsed
            .port()
            .unwrap_or_else(|| tls::default_port_for_scheme(parsed.scheme())),
    )
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn spawn_loopback_route(
    lookup: &WorkspaceLookup,
    local_port: u16,
    remote_port: u16,
) -> Result<Child, String> {
    spawn_remote_port_forward(lookup, local_port, remote_port)
}

fn wait_for_local_route(local_port: u16, child: &mut Child) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < ROUTER_ATTACH_TIMEOUT {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to inspect browser loopback route: {error}"))?
        {
            return Err(format!(
                "browser loopback route exited early with status {status}"
            ));
        }

        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{local_port}")
                .parse()
                .map_err(|error| format!("invalid browser loopback address: {error}"))?,
            Duration::from_millis(100),
        )
        .is_ok()
        {
            return Ok(());
        }
        std::thread::sleep(ROUTER_ATTACH_RETRY_INTERVAL);
    }

    Err("timed out waiting for browser loopback route".to_string())
}

fn find_free_local_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to allocate browser loopback port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("failed to read browser loopback port: {error}"))
}
