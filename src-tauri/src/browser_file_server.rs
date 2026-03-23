use crate::files;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use tauri::webview::Url;

const WORKSPACE_FILE_LOGICAL_HOST: &str = "workspace-file.silo";
const WORKSPACE_FILE_RESOLVED_HOST: &str = "workspace-file.localhost";
const WORKSPACE_FILE_ROUTE_SEGMENT: &str = "w";

#[derive(Clone)]
pub struct BrowserFileServerManager {
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceFileRequest {
    workspace: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpRequest {
    headers: HashMap<String, String>,
    method: String,
    target: String,
}

pub struct HttpResponse {
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub reason: &'static str,
    pub status: u16,
}

impl BrowserFileServerManager {
    pub fn new() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("failed to bind workspace file server: {error}"))?;
        let port = listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("failed to read workspace file server port: {error}"))?;

        thread::Builder::new()
            .name("workspace-file-server".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            thread::spawn(move || {
                                if let Err(error) = handle_connection(stream) {
                                    log::warn!("workspace file server request failed: {error}");
                                }
                            });
                        }
                        Err(error) => {
                            log::warn!("workspace file server accept failed: {error}");
                        }
                    }
                }
            })
            .map_err(|error| format!("failed to start workspace file server: {error}"))?;

        Ok(Self { port })
    }

    pub fn rewrite_workspace_file_url(&self, logical_url: &str) -> Result<Option<String>, String> {
        let Some(request) = parse_workspace_file_logical_url(logical_url) else {
            return Ok(None);
        };

        build_workspace_file_url(
            "http",
            WORKSPACE_FILE_RESOLVED_HOST,
            Some(self.port),
            &request.workspace,
            &request.path,
        )
        .map(Some)
        .map_err(|error| format!("failed to build workspace file url: {error}"))
    }

    pub fn logical_url_for_resolved_url(&self, resolved_url: &str) -> Option<String> {
        let request = parse_workspace_file_resolved_url(self.port, resolved_url)?;
        workspace_file_logical_url(&request.workspace, &request.path).ok()
    }
}

pub fn workspace_file_logical_url(workspace: &str, path: &str) -> Result<String, String> {
    build_workspace_file_url("https", WORKSPACE_FILE_LOGICAL_HOST, None, workspace, path)
        .map_err(|error| format!("failed to build logical workspace file url: {error}"))
}

pub fn workspace_file_display_name_from_url(url: &str) -> Option<String> {
    let request = parse_workspace_file_logical_url(url)?;
    Some(files::file_display_name(&request.path))
}

fn handle_connection(mut stream: TcpStream) -> Result<(), String> {
    let request = read_http_request(&stream)?;
    let response = build_response(&request);
    write_http_response(&mut stream, &request.method, response)
}

fn build_response(request: &HttpRequest) -> HttpResponse {
    if request.method != "GET" && request.method != "HEAD" {
        return text_response(405, "Method Not Allowed", "method not allowed");
    }

    let Some(file_request) = parse_workspace_file_resolved_target(&request.target) else {
        return text_response(404, "Not Found", "not found");
    };

    let Some(content_type) = files::browser_renderable_content_type(&file_request.path) else {
        return text_response(415, "Unsupported Media Type", "unsupported media type");
    };

    let asset = match tauri::async_runtime::block_on(files::read_workspace_file_asset(
        &file_request.workspace,
        &file_request.path,
    )) {
        Ok(Some(asset)) => asset,
        Ok(None) => return text_response(404, "Not Found", "file not found"),
        Err(error) => {
            log::warn!(
                "workspace file server failed workspace={} path={}: {}",
                file_request.workspace,
                file_request.path,
                error
            );
            let status = if error.contains("does not support browser rendering") {
                415
            } else if error.contains("not ready") {
                503
            } else {
                500
            };
            let reason = match status {
                415 => "Unsupported Media Type",
                503 => "Service Unavailable",
                _ => "Internal Server Error",
            };
            return text_response(status, reason, &error);
        }
    };

    match build_asset_response(&asset.bytes, content_type, request.headers.get("range")) {
        Ok(response) => response,
        Err(response) => response,
    }
}

fn build_asset_response(
    bytes: &[u8],
    content_type: &'static str,
    range_header: Option<&String>,
) -> Result<HttpResponse, HttpResponse> {
    let (status, reason, body, content_range) =
        match resolve_range(bytes, range_header).map_err(range_not_satisfiable_response)? {
            Some((start, end)) => (
                206,
                "Partial Content",
                bytes[start..=end].to_vec(),
                Some(format!("bytes {start}-{end}/{}", bytes.len())),
            ),
            None => (200, "OK", bytes.to_vec(), None),
        };

    let mut headers = vec![
        ("Cache-Control".to_string(), "no-store".to_string()),
        ("Connection".to_string(), "close".to_string()),
        ("Accept-Ranges".to_string(), "bytes".to_string()),
        ("Content-Disposition".to_string(), "inline".to_string()),
        ("Content-Length".to_string(), body.len().to_string()),
        ("Content-Type".to_string(), content_type.to_string()),
    ];
    if let Some(content_range) = content_range {
        headers.push(("Content-Range".to_string(), content_range));
    }

    Ok(HttpResponse {
        body,
        headers,
        reason,
        status,
    })
}

fn range_not_satisfiable_response(_: ()) -> HttpResponse {
    HttpResponse {
        body: Vec::new(),
        headers: vec![
            ("Cache-Control".to_string(), "no-store".to_string()),
            ("Connection".to_string(), "close".to_string()),
            ("Content-Range".to_string(), "bytes */0".to_string()),
        ],
        reason: "Range Not Satisfiable",
        status: 416,
    }
}

fn resolve_range(
    bytes: &[u8],
    range_header: Option<&String>,
) -> Result<Option<(usize, usize)>, ()> {
    let Some(range_header) = range_header else {
        return Ok(None);
    };
    let Some(range_value) = range_header.trim().strip_prefix("bytes=") else {
        return Err(());
    };
    if range_value.contains(',') {
        return Err(());
    }

    let len = bytes.len();
    if len == 0 {
        return Err(());
    }

    let (start_raw, end_raw) = range_value.split_once('-').ok_or(())?;
    let (start, end) = if start_raw.is_empty() {
        let suffix = end_raw.parse::<usize>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let clamped = suffix.min(len);
        (len - clamped, len - 1)
    } else {
        let start = start_raw.parse::<usize>().map_err(|_| ())?;
        let end = if end_raw.is_empty() {
            len - 1
        } else {
            end_raw.parse::<usize>().map_err(|_| ())?
        };
        (start, end)
    };

    if start >= len || start > end {
        return Err(());
    }

    Ok(Some((start, end.min(len - 1))))
}

fn read_http_request(stream: &TcpStream) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| format!("failed to clone workspace file socket: {error}"))?,
    );

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| format!("failed to read request line: {error}"))?;
    if request_line.trim().is_empty() {
        return Err("workspace file request line was empty".to_string());
    }

    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "workspace file request method missing".to_string())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "workspace file request target missing".to_string())?
        .to_string();

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read workspace file header: {error}"))?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    Ok(HttpRequest {
        headers,
        method,
        target,
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    method: &str,
    response: HttpResponse,
) -> Result<(), String> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, response.reason);
    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream
        .write_all(head.as_bytes())
        .map_err(|error| format!("failed to write workspace file response head: {error}"))?;
    if method != "HEAD" {
        stream
            .write_all(&response.body)
            .map_err(|error| format!("failed to write workspace file response body: {error}"))?;
    }
    stream
        .flush()
        .map_err(|error| format!("failed to flush workspace file response: {error}"))?;
    Ok(())
}

fn text_response(status: u16, reason: &'static str, body: &str) -> HttpResponse {
    HttpResponse {
        body: body.as_bytes().to_vec(),
        headers: vec![
            ("Cache-Control".to_string(), "no-store".to_string()),
            ("Connection".to_string(), "close".to_string()),
            ("Content-Length".to_string(), body.len().to_string()),
            (
                "Content-Type".to_string(),
                "text/plain; charset=utf-8".to_string(),
            ),
        ],
        reason,
        status,
    }
}

fn build_workspace_file_url(
    scheme: &str,
    host: &str,
    port: Option<u16>,
    workspace: &str,
    path: &str,
) -> Result<String, String> {
    let path = files::normalize_repo_relative_path(path)?;
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let mut url = Url::parse(&format!("{scheme}://{authority}"))
        .map_err(|error| format!("failed to create workspace file url: {error}"))?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| "workspace file url cannot be a base".to_string())?;
    segments.push(WORKSPACE_FILE_ROUTE_SEGMENT);
    segments.push(workspace);
    for segment in path.split('/') {
        segments.push(segment);
    }
    drop(segments);
    Ok(url.to_string())
}

fn parse_workspace_file_logical_url(url: &str) -> Option<WorkspaceFileRequest> {
    let parsed = Url::parse(url).ok()?;
    if parsed.host_str()? != WORKSPACE_FILE_LOGICAL_HOST {
        return None;
    }
    parse_workspace_file_path(&parsed)
}

fn parse_workspace_file_resolved_url(port: u16, url: &str) -> Option<WorkspaceFileRequest> {
    let parsed = Url::parse(url).ok()?;
    if parsed.host_str()? != WORKSPACE_FILE_RESOLVED_HOST {
        return None;
    }
    if parsed.port() != Some(port) {
        return None;
    }
    parse_workspace_file_path(&parsed)
}

fn parse_workspace_file_resolved_target(target: &str) -> Option<WorkspaceFileRequest> {
    let parsed = Url::parse(&format!("http://{WORKSPACE_FILE_RESOLVED_HOST}{target}")).ok()?;
    parse_workspace_file_path(&parsed)
}

fn parse_workspace_file_path(url: &Url) -> Option<WorkspaceFileRequest> {
    let mut segments = url.path_segments()?;
    if segments.next()? != WORKSPACE_FILE_ROUTE_SEGMENT {
        return None;
    }

    let workspace = segments.next()?.to_string();
    if workspace.is_empty() {
        return None;
    }

    let path = segments.collect::<Vec<_>>().join("/");
    if path.is_empty() {
        return None;
    }
    let path = files::normalize_repo_relative_path(&path).ok()?;
    Some(WorkspaceFileRequest { workspace, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_file_logical_url_round_trips() {
        let logical =
            workspace_file_logical_url("demo-workspace", "images/diagram.png").expect("url");
        assert_eq!(
            logical,
            "https://workspace-file.silo/w/demo-workspace/images/diagram.png"
        );
        let parsed = parse_workspace_file_logical_url(&logical).expect("request");
        assert_eq!(parsed.workspace, "demo-workspace");
        assert_eq!(parsed.path, "images/diagram.png");
    }

    #[test]
    fn workspace_file_resolved_url_round_trips() {
        let manager = BrowserFileServerManager { port: 64601 };
        let logical =
            workspace_file_logical_url("demo-workspace", "docs/report.final.pdf").expect("url");
        let resolved = manager
            .rewrite_workspace_file_url(&logical)
            .expect("resolved url")
            .expect("workspace file url");
        assert_eq!(
            resolved,
            "http://workspace-file.localhost:64601/w/demo-workspace/docs/report.final.pdf"
        );
        assert_eq!(
            manager.logical_url_for_resolved_url(&resolved).as_deref(),
            Some(logical.as_str())
        );
    }

    #[test]
    fn workspace_file_resolved_target_round_trips() {
        let parsed = parse_workspace_file_resolved_target(
            "/w/demo-workspace/specs/act/docs/ACT_Technical_Manual.pdf",
        )
        .expect("request");
        assert_eq!(parsed.workspace, "demo-workspace");
        assert_eq!(parsed.path, "specs/act/docs/ACT_Technical_Manual.pdf");
    }

    #[test]
    fn workspace_file_display_name_uses_file_name() {
        let logical =
            workspace_file_logical_url("demo-workspace", "docs/report.final.pdf").expect("url");
        assert_eq!(
            workspace_file_display_name_from_url(&logical).as_deref(),
            Some("report.final.pdf")
        );
    }

    #[test]
    fn resolve_range_supports_open_ended_range() {
        let bytes = b"abcdef";
        let range = resolve_range(bytes, Some(&"bytes=2-".to_string())).expect("range");
        assert_eq!(range, Some((2, 5)));
    }

    #[test]
    fn resolve_range_supports_suffix_range() {
        let bytes = b"abcdef";
        let range = resolve_range(bytes, Some(&"bytes=-3".to_string())).expect("range");
        assert_eq!(range, Some((3, 5)));
    }
}
