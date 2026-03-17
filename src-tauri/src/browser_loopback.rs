use crate::router::RouterManager;
use crate::workspaces;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use std::convert::Infallible;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use url::form_urlencoded;

const LOOPBACK_RESOLVER_ENV: &str = "SILO_BROWSER_LOOPBACK_RESOLVER_URL";

type ResolverBody = Full<Bytes>;

#[derive(Clone)]
pub struct BrowserLoopbackManager {
    loopback_router: RouterManager,
    started: Arc<AtomicBool>,
    listen_addr: Arc<Mutex<Option<std::net::SocketAddr>>>,
}

impl BrowserLoopbackManager {
    pub fn new(loopback_router: RouterManager) -> Self {
        Self {
            loopback_router,
            started: Arc::new(AtomicBool::new(false)),
            listen_addr: Arc::default(),
        }
    }

    pub fn ensure_started(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let listener = StdTcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("failed to bind browser loopback resolver: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("failed to configure browser loopback resolver: {error}"))?;
        let listen_addr = listener
            .local_addr()
            .map_err(|error| format!("failed to read browser loopback resolver address: {error}"))?;

        {
            let mut current = self
                .listen_addr
                .lock()
                .map_err(|_| "browser loopback resolver address lock poisoned".to_string())?;
            current.replace(listen_addr);
        }

        std::env::set_var(
            LOOPBACK_RESOLVER_ENV,
            format!("http://127.0.0.1:{}/rewrite", listen_addr.port()),
        );

        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = manager.run(listener).await {
                log::error!("browser loopback resolver stopped: {error}");
                manager.started.store(false, Ordering::SeqCst);
            }
        });

        log::info!(
            "browser loopback resolver listening at http://127.0.0.1:{}/rewrite",
            listen_addr.port()
        );
        Ok(())
    }

    async fn run(&self, listener: StdTcpListener) -> Result<(), String> {
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|error| format!("failed to create async browser loopback listener: {error}"))?;

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|error| format!("failed to accept browser loopback connection: {error}"))?;
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |request| {
                    let manager = manager.clone();
                    async move { manager.handle_request(request).await }
                });
                let builder = ServerBuilder::new(TokioExecutor::new());
                if let Err(error) = builder.serve_connection(io, service).await {
                    log::debug!("browser loopback resolver connection closed: {error}");
                }
            });
        }
    }

    async fn handle_request(
        &self,
        request: Request<Incoming>,
    ) -> Result<Response<ResolverBody>, Infallible> {
        if request.method() != Method::GET {
            return Ok(text_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "browser loopback resolver only supports GET",
            ));
        }

        if request.uri().path() != "/rewrite" {
            return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
        }

        let query = request.uri().query().unwrap_or_default();
        let mut label = None;
        let mut original_url = None;
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "label" => label = Some(value.into_owned()),
                "url" => original_url = Some(value.into_owned()),
                _ => {}
            }
        }

        let Some(label) = label else {
            return Ok(text_response(StatusCode::BAD_REQUEST, "missing label query parameter"));
        };
        let Some(original_url) = original_url else {
            return Ok(text_response(StatusCode::BAD_REQUEST, "missing url query parameter"));
        };

        match self.rewrite_loopback_url(&label, &original_url).await {
            Ok(Some(rewritten)) => Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from(rewritten)))
                .expect("browser loopback resolver response should be valid")),
            Ok(None) => Ok(Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Full::new(Bytes::new()))
                .expect("browser loopback resolver empty response should be valid")),
            Err(error) => Ok(text_response(StatusCode::BAD_GATEWAY, error)),
        }
    }

    async fn rewrite_loopback_url(
        &self,
        label: &str,
        original_url: &str,
    ) -> Result<Option<String>, String> {
        let Some(workspace) = browser_label_workspace(label) else {
            return Ok(None);
        };

        if self
            .loopback_router
            .logical_url_for_reported_url(workspace, original_url)
            .is_some()
        {
            return Ok(None);
        }

        let lookup = workspaces::find_workspace(workspace).await?;
        let rewritten = self.loopback_router.rewrite_loopback_url(&lookup, original_url)?;
        if let Some(rewritten) = &rewritten {
            log::info!(
                "browser loopback rewrote request workspace={} label={} from={} to={}",
                workspace,
                label,
                original_url,
                rewritten
            );
        }
        Ok(rewritten)
    }
}

fn browser_label_workspace(label: &str) -> Option<&str> {
    let mut parts = label.splitn(3, ':');
    if parts.next()? != "browser" {
        return None;
    }
    parts.next()
}

fn text_response(status: StatusCode, message: impl Into<String>) -> Response<ResolverBody> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from(message.into())))
        .expect("browser loopback resolver error response should be valid")
}
