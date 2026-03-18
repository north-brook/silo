use crate::router::RouterManager;
use crate::workspaces::{self, WorkspaceLookup};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct BrowserLoopbackManager {
    loopback_router: RouterManager,
    workspace_lookups: Arc<RwLock<HashMap<String, WorkspaceLookup>>>,
}

impl BrowserLoopbackManager {
    pub fn new(loopback_router: RouterManager) -> Self {
        Self {
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

        let lookup = tauri::async_runtime::block_on(workspaces::find_workspace(workspace))?;
        self.cache_workspace_lookup(&lookup);
        Ok(lookup)
    }
}

fn browser_label_workspace(label: &str) -> Option<&str> {
    let mut parts = label.splitn(3, ':');
    if parts.next()? != "browser" {
        return None;
    }
    parts.next()
}
