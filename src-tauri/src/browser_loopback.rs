use crate::browser_file_server::BrowserFileServerManager;
use crate::router::RouterManager;
use crate::workspaces::WorkspaceLookup;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct BrowserLoopbackManager {
    file_server: BrowserFileServerManager,
    loopback_router: RouterManager,
    workspace_lookups: Arc<RwLock<HashMap<String, WorkspaceLookup>>>,
}

impl BrowserLoopbackManager {
    pub fn new(loopback_router: RouterManager, file_server: BrowserFileServerManager) -> Self {
        Self {
            file_server,
            loopback_router,
            workspace_lookups: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn cache_workspace_lookup(&self, lookup: &WorkspaceLookup) {
        if let Ok(mut workspaces) = self.workspace_lookups.write() {
            workspaces.insert(lookup.workspace.name().to_string(), lookup.clone());
        }
    }

    pub fn rewrite_loopback_url(
        &self,
        webview_label: &str,
        original_url: &str,
    ) -> Result<Option<String>, String> {
        let Some(workspace) = browser_label_workspace(webview_label) else {
            return Ok(None);
        };

        if self
            .file_server
            .logical_url_for_resolved_url(original_url)
            .is_some()
        {
            return Ok(None);
        }

        if self
            .loopback_router
            .logical_url_for_reported_url(workspace, original_url)
            .is_some()
        {
            return Ok(None);
        }

        let lookup = self.lookup_workspace(workspace)?;
        let rewritten = self
            .loopback_router
            .rewrite_loopback_url(&lookup, original_url)?;
        if let Some(rewritten) = &rewritten {
            log::debug!(
                "browser loopback rewrote request workspace={} label={} from={} to={}",
                workspace,
                webview_label,
                original_url,
                rewritten
            );
        }
        Ok(rewritten)
    }

    fn lookup_workspace(&self, workspace: &str) -> Result<WorkspaceLookup, String> {
        if let Ok(workspaces) = self.workspace_lookups.read() {
            if let Some(lookup) = workspaces.get(workspace) {
                return Ok(lookup.clone());
            }
        }

        Err(format!(
            "workspace lookup unavailable for browser loopback rewrite: {workspace}"
        ))
    }
}

fn browser_label_workspace(label: &str) -> Option<&str> {
    let mut parts = label.splitn(3, ':');
    if parts.next()? != "browser" {
        return None;
    }
    parts.next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_file_server::workspace_file_logical_url;

    #[test]
    fn rewrite_loopback_url_skips_workspace_file_server_requests() {
        let file_server = BrowserFileServerManager::new().expect("file server");
        let manager = BrowserLoopbackManager::new(RouterManager::default(), file_server.clone());
        let logical =
            workspace_file_logical_url("demo-workspace", "docs/report.final.pdf").expect("url");
        let resolved = file_server
            .rewrite_workspace_file_url(&logical)
            .expect("resolved url")
            .expect("workspace file url");

        let rewritten = manager
            .rewrite_loopback_url("browser:demo-workspace:tab-1", &resolved)
            .expect("rewrite result");

        assert_eq!(rewritten, None);
    }

    #[test]
    fn rewrite_loopback_url_requires_cached_workspace_lookup() {
        let file_server = BrowserFileServerManager::new().expect("file server");
        let manager = BrowserLoopbackManager::new(RouterManager::default(), file_server);

        let error = manager
            .rewrite_loopback_url("browser:demo-workspace:tab-1", "http://localhost:3000")
            .expect_err("missing cached workspace should fail");

        assert!(error.contains("workspace lookup unavailable"));
    }
}
