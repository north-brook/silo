// Copyright 2019-2024 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
  borrow::Cow,
  io::{Cursor, Read},
  sync::{Arc, OnceLock},
};

use cef::{rc::*, *};
use cookie::SameSite;
use dioxus_debug_cell::RefCell;
use html5ever::{LocalName, interface::QualName, namespace_url, ns};
use http::{
  HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
  header::{
    ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST,
  },
};
use kuchiki::NodeRef;
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use reqwest::blocking::Client;
use tauri_runtime::webview::UriSchemeProtocolHandler;
use tauri_utils::{
  config::{Csp, CspDirectiveSources},
  html::{parse as parse_html, serialize_node},
};
use url::Url;

use super::CefInitScript;
type HttpResponse = Arc<RefCell<Option<http::Response<Cursor<Vec<u8>>>>>>;

enum LoopbackResolution {
  NotHandled,
  Rewritten(String),
  Error(String),
}

#[derive(Clone)]
struct LoopbackFetchRequest {
  original_url: String,
  rewritten_url: String,
  method: Method,
  headers: HeaderMap,
  body: Vec<u8>,
}

fn csp_inject_initialization_scripts_hashes(
  existing_csp: String,
  initialization_scripts: &[CefInitScript],
) -> String {
  if initialization_scripts.is_empty() {
    return existing_csp;
  }

  let script_hashes: Vec<String> = initialization_scripts
    .iter()
    .map(|s| s.hash.clone())
    .collect();

  if script_hashes.is_empty() {
    return existing_csp;
  }

  let mut csp_map: std::collections::HashMap<String, CspDirectiveSources> =
    Csp::Policy(existing_csp.to_string()).into();

  let script_src = csp_map
    .entry("script-src".to_string())
    .or_insert_with(|| CspDirectiveSources::List(vec!["'self'".to_string()]));
  script_src.extend(script_hashes);

  Csp::DirectiveMap(csp_map).to_string()
}

fn inject_scripts_into_html_body(
  body: &[u8],
  initialization_scripts: &[CefInitScript],
) -> Option<Vec<u8>> {
  let Ok(body_str) = std::str::from_utf8(body) else {
    return None;
  };

  let document = parse_html(body_str.to_string());

  let head = if let Ok(ref head_node) = document.select_first("head") {
    head_node.as_node().clone()
  } else {
    let head_node = NodeRef::new_element(
      QualName::new(None, ns!(html), LocalName::from("head")),
      None,
    );
    document.prepend(head_node.clone());
    head_node
  };

  for init_script in initialization_scripts.iter().rev() {
    let script_el = NodeRef::new_element(QualName::new(None, ns!(html), "script".into()), None);
    script_el.append(NodeRef::new_text(init_script.script.script.as_str()));
    head.prepend(script_el);
  }

  Some(serialize_node(&document))
}

wrap_resource_request_handler! {
  pub struct WebResourceRequestHandler {
    webview_label: String,
    initialization_scripts: Arc<Vec<CefInitScript>>,
  }

  impl ResourceRequestHandler {
    fn on_before_resource_load(
      &self,
      _browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      _request: Option<&mut Request>,
      _callback: Option<&mut Callback>,
    ) -> ReturnValue {
      sys::cef_return_value_t::RV_CONTINUE.into()
    }

    fn resource_handler(
      &self,
      _browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      request: Option<&mut Request>,
    ) -> Option<ResourceHandler> {
      let request = request?;
      let original_url = CefString::from(&request.url()).to_string();

      let resolution = resolve_loopback_request(&self.webview_label, &original_url);
      match resolution {
        LoopbackResolution::NotHandled => None,
        LoopbackResolution::Error(error) => Some(LoopbackResourceHandler::new(
          None,
          Arc::new(RefCell::new(Some(error_response(error)))),
        )),
        LoopbackResolution::Rewritten(rewritten_url) => {
          let headers = get_request_headers(request);
          let body = read_request_body(request);
          let method_str = CefString::from(&request.method()).to_string();
          let method = Method::from_bytes(method_str.as_bytes()).unwrap_or(Method::GET);
          Some(LoopbackResourceHandler::new(
            Some(LoopbackFetchRequest {
              original_url,
              rewritten_url,
              method,
              headers,
              body,
            }),
            Arc::new(RefCell::new(None)),
          ))
        }
      }
    }
  }
}

wrap_request_handler! {
  pub struct WebRequestHandler {
    webview_label: String,
    initialization_scripts: Arc<Vec<CefInitScript>>,
    navigation_handler: Option<Arc<tauri_runtime::webview::NavigationHandler>>,
  }

  impl RequestHandler {
    fn on_before_browse(
      &self,
      _browser: Option<&mut Browser>,
      frame: Option<&mut Frame>,
      request: Option<&mut Request>,
      _user_gesture: ::std::os::raw::c_int,
      _is_redirect: ::std::os::raw::c_int,
    ) -> ::std::os::raw::c_int {
      let Some(frame) = frame else {
        return 0;
      };
      if frame.is_main() == 0 {
        return 0;
      }
      let Some(handler) = &self.navigation_handler else {
        return 0;
      };
      let Some(request) = request else {
        return 0;
      };

      let url_str = CefString::from(&request.url()).to_string();
      let Ok(url) = url::Url::parse(&url_str) else {
        return 0;
      };
      let should_navigate = handler(&url);
      if should_navigate {
        0
      } else {
        1
      }
    }

    fn resource_request_handler(
      &self,
      _browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      _request: Option<&mut Request>,
      _is_navigation: ::std::os::raw::c_int,
      _is_download: ::std::os::raw::c_int,
      _request_initiator: Option<&CefString>,
      _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
    ) -> Option<ResourceRequestHandler> {
      Some(WebResourceRequestHandler::new(
        self.webview_label.clone(),
        self.initialization_scripts.clone(),
      ))
    }
  }
}

wrap_resource_handler! {
  pub struct WebResourceHandler {
    webview_label: String,
    handler: Arc<Box<UriSchemeProtocolHandler>>,
    initialization_scripts: Arc<Vec<CefInitScript>>,
    response: HttpResponse,
  }

  impl ResourceHandler {
    fn process_request(
      &self,
      request: Option<&mut Request>,
      callback: Option<&mut Callback>,
    ) -> ::std::os::raw::c_int {
      let Some(request) = request else { return 0 };
      let Some(callback) = callback else { return 0 };

      let url = CefString::from(&request.url()).to_string();
      let url = Url::parse(&url).ok();

      if let Some(url) = url {
        let callback = ThreadSafe(callback.clone());
        let response_store = ThreadSafe(self.response.clone());
        let initialization_scripts = self.initialization_scripts.clone();
        let responder = Box::new(move |response: http::Response<Cow<'static, [u8]>>| {
          let content_type = response.headers().get(CONTENT_TYPE);
          let is_html = content_type
            .and_then(|ct| ct.to_str().ok())
            .map(|ct| ct.to_lowercase().starts_with("text/html"))
            .unwrap_or(false);

          let (parts, body) = response.into_parts();
          let body_bytes = body.into_owned();

          let modified_body = if is_html {
            inject_scripts_into_html_body(&body_bytes, &initialization_scripts)
              .unwrap_or(body_bytes)
          } else {
            body_bytes
          };

          let mut response = http::Response::from_parts(parts, Cursor::new(modified_body));

          let csp = response
            .headers_mut()
            .get_mut(CONTENT_SECURITY_POLICY);

          if let Some(csp) = csp {
            let csp_string = csp.to_str().unwrap().to_string();
            let new_csp = csp_inject_initialization_scripts_hashes(
              csp_string,
              &initialization_scripts,
            );
            *csp = HeaderValue::from_str(&new_csp).unwrap();
          }

          response_store.into_owned().borrow_mut().replace(response);

          let callback = callback.into_owned();
          callback.cont();
        });

        let label = self.webview_label.clone();
        let handler = self.handler.clone();

        let data = read_request_body(request);
        let headers = get_request_headers(request);
        let method_str = CefString::from(&request.method()).to_string();
        let method = Method::from_bytes(method_str.as_bytes())
          .unwrap_or(Method::GET);

        std::thread::spawn(move || {
          let mut http_request = http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .body(data)
            .unwrap();
          *http_request.headers_mut() = headers;
          (**handler)(&label, http_request, responder);
        });
        1
      } else {
        0
      }
    }

    fn read(
      &self,
      data_out: *mut u8,
      bytes_to_read: ::std::os::raw::c_int,
      bytes_read: Option<&mut ::std::os::raw::c_int>,
      _callback: Option<&mut ResourceReadCallback>,
    ) -> ::std::os::raw::c_int {
      read_response_bytes(&self.response, data_out, bytes_to_read, bytes_read)
    }

    fn response_headers(
      &self,
      response: Option<&mut Response>,
      response_length: Option<&mut i64>,
      redirect_url: Option<&mut CefString>,
    ) {
      write_response_headers(&self.response, response, response_length, redirect_url, true);
    }
  }
}

wrap_resource_handler! {
  pub struct LoopbackResourceHandler {
    fetch: Option<LoopbackFetchRequest>,
    response: HttpResponse,
  }

  impl ResourceHandler {
    fn process_request(
      &self,
      _request: Option<&mut Request>,
      callback: Option<&mut Callback>,
    ) -> ::std::os::raw::c_int {
        let Some(callback) = callback else { return 0 };

      if let Some(fetch) = self.fetch.clone() {
        let callback = ThreadSafe(callback.clone());
        let response_store = ThreadSafe(self.response.clone());
        loopback_fetch_pool().spawn(move || {
          let response = fetch_loopback_response(fetch);
          response_store.into_owned().borrow_mut().replace(response);
          callback.into_owned().cont();
        });
      } else {
        callback.cont();
      }

      1
    }

    fn read(
      &self,
      data_out: *mut u8,
      bytes_to_read: ::std::os::raw::c_int,
      bytes_read: Option<&mut ::std::os::raw::c_int>,
      _callback: Option<&mut ResourceReadCallback>,
    ) -> ::std::os::raw::c_int {
      read_response_bytes(&self.response, data_out, bytes_to_read, bytes_read)
    }

    fn response_headers(
      &self,
      response: Option<&mut Response>,
      response_length: Option<&mut i64>,
      redirect_url: Option<&mut CefString>,
    ) {
      write_response_headers(&self.response, response, response_length, redirect_url, false);
    }
  }
}

wrap_scheme_handler_factory! {
  pub struct UriSchemeHandlerFactory {
    registry: super::SchemeHandlerRegistry,
    scheme: String,
  }

  impl SchemeHandlerFactory {
    fn create(
      &self,
      browser: Option<&mut Browser>,
      _frame: Option<&mut Frame>,
      _scheme_name: Option<&CefString>,
      _request: Option<&mut Request>,
    ) -> Option<ResourceHandler> {
      let browser = browser?;
      let id = browser.identifier();

      let (webview_label, handler, initialization_scripts) = self
        .registry
        .lock()
        .unwrap()
        .get(&(id, self.scheme.clone()))
        .cloned()?;

      Some(WebResourceHandler::new(
        webview_label,
        handler,
        initialization_scripts,
        Arc::new(RefCell::new(None)),
      ))
    }
  }
}

struct ThreadSafe<T>(T);

impl<T> ThreadSafe<T> {
  fn into_owned(self) -> T {
    self.0
  }
}

unsafe impl<T> Send for ThreadSafe<T> {}
unsafe impl<T> Sync for ThreadSafe<T> {}

fn resolve_loopback_request(webview_label: &str, original_url: &str) -> LoopbackResolution {
  if !webview_label.starts_with("browser:") {
    return LoopbackResolution::NotHandled;
  }

  let Ok(url) = Url::parse(original_url) else {
    return LoopbackResolution::NotHandled;
  };
  let Some(host) = url.host_str() else {
    return LoopbackResolution::NotHandled;
  };
  if !matches!(url.scheme(), "http" | "https") || !is_loopback_host(host) {
    return LoopbackResolution::NotHandled;
  }

  match crate::resolve_loopback_request_url(webview_label, original_url) {
    Ok(Some(rewritten)) if !rewritten.trim().is_empty() => LoopbackResolution::Rewritten(rewritten),
    Ok(_) => LoopbackResolution::NotHandled,
    Err(error) => LoopbackResolution::Error(error),
  }
}

fn fetch_loopback_response(fetch: LoopbackFetchRequest) -> http::Response<Cursor<Vec<u8>>> {
  let mut headers = fetch.headers;
  headers.remove(CONTENT_LENGTH);
  headers.remove(ACCEPT_ENCODING);
  headers.remove(CONNECTION);
  headers.remove(HOST);
  headers.remove(HeaderName::from_static("forwarded"));
  headers.remove(HeaderName::from_static("x-forwarded-for"));
  headers.remove(HeaderName::from_static("x-forwarded-host"));
  headers.remove(HeaderName::from_static("x-forwarded-port"));
  headers.remove(HeaderName::from_static("x-forwarded-proto"));
  headers.remove(HeaderName::from_static("x-forwarded-server"));

  if let Some(host_header) = original_authority_header_value(&fetch.original_url) {
    headers.insert(HOST, host_header);
  }

  let mut request = loopback_http_client().request(fetch.method, &fetch.rewritten_url);
  for (name, value) in &headers {
    request = request.header(name, value);
  }
  if !fetch.body.is_empty() {
    request = request.body(fetch.body);
  }

  let upstream_response = match request.send() {
    Ok(response) => response,
    Err(error) => {
      return error_response(format!(
        "failed to fetch rewritten loopback request {} -> {}: {error}",
        fetch.original_url, fetch.rewritten_url
      ));
    }
  };

  let status = upstream_response.status();
  let headers = upstream_response.headers().clone();
  sync_loopback_response_cookies(&fetch.original_url, &headers);
  let body = match upstream_response.bytes() {
    Ok(body) => body.to_vec(),
    Err(error) => {
      return error_response(format!(
        "failed to read rewritten loopback response {} -> {}: {error}",
        fetch.original_url, fetch.rewritten_url
      ));
    }
  };

  let mut response = http::Response::builder().status(status.as_u16());
  for (name, value) in &headers {
    response = response.header(name, value);
  }
  response
    .body(Cursor::new(body))
    .unwrap_or_else(|error| error_response(format!("failed to build loopback response: {error}")))
}

fn loopback_http_client() -> &'static Client {
  static CLIENT: OnceLock<Client> = OnceLock::new();
  CLIENT.get_or_init(|| {
    Client::builder()
      .redirect(reqwest::redirect::Policy::none())
      .danger_accept_invalid_certs(true)
      .no_proxy()
      .pool_max_idle_per_host(32)
      .tcp_nodelay(true)
      .build()
      .expect("loopback http client should be valid")
  })
}

fn loopback_fetch_pool() -> &'static ThreadPool {
  static POOL: OnceLock<ThreadPool> = OnceLock::new();
  POOL.get_or_init(|| {
    let worker_count = std::thread::available_parallelism()
      .map(|count| count.get().saturating_mul(2).clamp(4, 32))
      .unwrap_or(8);
    ThreadPoolBuilder::new()
      .num_threads(worker_count)
      .thread_name(|index| format!("silo-loopback-{index}"))
      .build()
      .expect("loopback fetch pool should be valid")
  })
}

fn sync_loopback_response_cookies(original_url: &str, headers: &HeaderMap) {
  let Some(manager) = cef::cookie_manager_get_global_manager(None) else {
    return;
  };
  let cookie_url = cef::CefString::from(original_url);

  for set_cookie in headers.get_all(http::header::SET_COOKIE) {
    let Ok(set_cookie) = set_cookie.to_str() else {
      continue;
    };
    let Ok(parsed) = cookie::Cookie::parse(set_cookie) else {
      log::debug!("failed to parse loopback Set-Cookie header for {original_url}");
      continue;
    };

    let mut cef_cookie = cef::Cookie {
      name: cef::CefString::from(parsed.name()),
      value: cef::CefString::from(parsed.value()),
      ..Default::default()
    };

    if let Some(domain) = parsed.domain() {
      cef_cookie.domain = cef::CefString::from(domain);
    }
    if let Some(path) = parsed.path() {
      cef_cookie.path = cef::CefString::from(path);
    }
    if parsed.secure().unwrap_or(false) {
      cef_cookie.secure = 1;
    }
    if parsed.http_only().unwrap_or(false) {
      cef_cookie.httponly = 1;
    }
    cef_cookie.same_site = match parsed.same_site() {
      Some(SameSite::Strict) => cef::CookieSameSite::STRICT_MODE,
      Some(SameSite::Lax) => cef::CookieSameSite::LAX_MODE,
      Some(SameSite::None) => cef::CookieSameSite::NO_RESTRICTION,
      None => cef::CookieSameSite::UNSPECIFIED,
    };

    if let Some(expires) = parsed.expires().and_then(|expiration| expiration.datetime()) {
      let mut cef_time = cef::Time::default();
      if cef::time_from_doublet(expires.unix_timestamp_nanos() as f64 / 1_000_000_000.0, Some(&mut cef_time))
        != 0
      {
        let mut basetime = cef::Basetime::default();
        if cef::time_to_basetime(Some(&cef_time), Some(&mut basetime)) != 0 {
          cef_cookie.has_expires = 1;
          cef_cookie.expires = basetime;
        }
      }
    }

    let _ = manager.set_cookie(
      Some(&cookie_url),
      Some(&cef_cookie),
      Option::<&mut cef::SetCookieCallback>::None,
    );
  }
}

fn original_authority_header_value(original_url: &str) -> Option<HeaderValue> {
  let original_url = Url::parse(original_url).ok()?;
  let host = original_url.host_str()?;
  let authority = match original_url.port() {
    Some(port) => format!("{host}:{port}"),
    None => host.to_string(),
  };
  HeaderValue::from_str(&authority).ok()
}

fn error_response(message: String) -> http::Response<Cursor<Vec<u8>>> {
  http::Response::builder()
    .status(StatusCode::BAD_GATEWAY.as_u16())
    .header(CONTENT_TYPE, "text/plain; charset=utf-8")
    .body(Cursor::new(message.into_bytes()))
    .expect("loopback error response should be valid")
}

fn is_loopback_host(host: &str) -> bool {
  matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn read_response_bytes(
  response_store: &HttpResponse,
  data_out: *mut u8,
  bytes_to_read: ::std::os::raw::c_int,
  bytes_read: Option<&mut ::std::os::raw::c_int>,
) -> ::std::os::raw::c_int {
  let Ok(bytes_to_read) = usize::try_from(bytes_to_read) else {
    return 0;
  };
  let data_out = unsafe { std::slice::from_raw_parts_mut(data_out, bytes_to_read) };
  let count = response_store
    .borrow_mut()
    .as_mut()
    .and_then(|response| response.body_mut().read(data_out).ok())
    .unwrap_or(0);
  if let Some(bytes_read) = bytes_read {
    let Ok(count) = count.try_into() else {
      return 0;
    };
    *bytes_read = count;
    if count > 0 {
      return 1;
    }
  }
  0
}

fn write_response_headers(
  response_store: &HttpResponse,
  response: Option<&mut Response>,
  response_length: Option<&mut i64>,
  redirect_url: Option<&mut CefString>,
  force_no_store: bool,
) {
  let (Some(response), Some(response_data)) = (response, &*response_store.borrow()) else {
    return;
  };

  response.set_status(response_data.status().as_u16() as i32);
  let mut content_type = None;

  for (name, value) in response_data.headers() {
    let Ok(value) = value.to_str() else {
      continue;
    };

    response.set_header_by_name(Some(&name.as_str().into()), Some(&value.into()), 0);

    if name == CONTENT_TYPE {
      content_type.replace(value.to_string());
    }
  }

  if force_no_store {
    response.set_header_by_name(
      Some(&"Cache-Control".into()),
      Some(&"no-store".into()),
      1,
    );
  }

  let mime_type = content_type
    .as_ref()
    .and_then(|t| t.split(';').next())
    .map(str::trim)
    .unwrap_or("text/plain");
  response.set_mime_type(Some(&mime_type.into()));

  if let Some(length) = response_length {
    *length = -1;
  }

  if let Some(redirect_url) = redirect_url {
    let _ = std::mem::take(redirect_url);
  }
}

fn read_request_body(request: &mut Request) -> Vec<u8> {
  let mut body = Vec::new();

  if let Some(post_data) = request.post_data() {
    let mut elements = vec![None; post_data.element_count()];
    post_data.elements(Some(&mut elements));
    for element in elements.into_iter().flatten() {
      match element.get_type().as_ref() {
        sys::cef_postdataelement_type_t::PDE_TYPE_BYTES => {
          let size = element.bytes_count();
          if size > 0 {
            let mut buf = vec![0u8; size];
            let copied = element.bytes(size, buf.as_mut_ptr());
            unsafe {
              buf.set_len(copied);
            }
            body.extend(buf);
          }
        }
        sys::cef_postdataelement_type_t::PDE_TYPE_FILE => {
          let file_path = CefString::from(&element.file()).to_string();
          if let Ok(mut file) = std::fs::File::open(&file_path) {
            use std::io::Read;
            let mut buf = Vec::new();
            if file.read_to_end(&mut buf).is_ok() {
              body.extend(buf);
            }
          }
        }
        _ => {}
      }
    }
  }

  body
}

fn get_request_headers(request: &mut Request) -> HeaderMap {
  let mut headers = HeaderMap::new();

  let mut map = CefStringMultimap::new();
  request.header_map(Some(&mut map));

  for (name, value) in map {
    for v in value {
      headers.append(
        HeaderName::from_bytes(name.as_bytes()).unwrap(),
        HeaderValue::from_str(&v).unwrap(),
      );
    }
  }

  headers
}
