#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{HeaderName, HeaderValue};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::routing::any;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing_subscriber::fmt::MakeWriter;

use crate::prompt_security_files::{FileSanitizer, FileSanitizerSettings};

pub(super) struct Seen {
    pub(super) method: String,
    pub(super) query: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
}

struct VendorState {
    upload: (StatusCode, String),
    polls: Mutex<VecDeque<(StatusCode, String)>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    delay: Duration,
}

pub(super) struct Vendor {
    pub(super) base: String,
    pub(super) seen: Arc<Mutex<Vec<Seen>>>,
    pub(super) _server: JoinSet<io::Result<()>>,
}

impl Vendor {
    pub(super) fn requests(&self, method: &str) -> Vec<Seen> {
        let seen = self.seen.lock().expect("seen");
        seen.iter()
            .filter(|request| request.method == method)
            .map(|request| Seen {
                method: request.method.clone(),
                query: request.query.clone(),
                headers: request.headers.clone(),
                body: request.body.clone(),
            })
            .collect()
    }
}

pub(super) const CREATED: &str = r#"{"status":"created"}"#;
pub(super) const IN_PROGRESS: &str = r#"{"status":"in progress"}"#;
pub(super) const DONE_ALLOW: &str = r#"{"status":"done","metadata":{"action":"allow"}}"#;
pub(super) const UPLOADED: &str = r#"{"jobId":"job-1"}"#;

pub(super) fn done(action: &str, content: &str) -> String {
    format!(
        r#"{{"status":"done","content":"{content}","metadata":{{"action":"{action}","violations":["secret","pii"]}}}}"#
    )
}

pub(super) async fn vendor(upload: &str, polls: &[String]) -> Vendor {
    vendor_with(StatusCode::OK, upload, polls, Duration::ZERO).await
}

pub(super) async fn vendor_with(
    upload_status: StatusCode,
    upload: &str,
    polls: &[String],
    delay: Duration,
) -> Vendor {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new(VendorState {
        upload: (upload_status, upload.to_owned()),
        polls: Mutex::new(
            polls
                .iter()
                .map(|body| (StatusCode::OK, body.clone()))
                .collect(),
        ),
        seen: Arc::clone(&seen),
        delay,
    });
    let router = Router::new()
        .route("/api/sanitizeFile", any(handle))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    Vendor {
        base,
        seen,
        _server: server,
    }
}

async fn handle(
    State(state): State<Arc<VendorState>>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, String) {
    state.seen.lock().expect("seen").push(Seen {
        method: method.to_string(),
        query: uri.query().unwrap_or_default().to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    str::from_utf8(value.as_bytes())
                        .expect("utf-8 header")
                        .to_owned(),
                )
            })
            .collect(),
        body: body.to_vec(),
    });
    sleep(state.delay).await;
    if method == Method::POST {
        return state.upload.clone();
    }
    let mut polls = state.polls.lock().expect("polls");
    if polls.len() > 1 {
        polls.pop_front().expect("poll")
    } else {
        polls.front().cloned().expect("poll")
    }
}

pub(super) fn sanitizer(base: &str) -> FileSanitizer {
    #[expect(
        clippy::disallowed_methods,
        reason = "tests talk to a local mock vendor without the shared HTTP client crate"
    )]
    let client = reqwest::Client::builder().build().expect("client");
    FileSanitizer {
        settings: FileSanitizerSettings {
            api_key: SecretString::from("vendor-key"),
            api_base: base.to_owned(),
            request_timeout: Duration::from_secs(5),
            file_sanitization_timeout: Duration::from_secs(10),
            fail_open: true,
            block_on_file_modify: true,
            max_poll_attempts: 30,
            poll_interval: Duration::from_millis(2),
            max_response_bytes: 1024 * 1024,
        },
        client,
    }
}

pub(super) fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

pub(super) fn header_map(entries: &[(&str, &str)]) -> HeaderMap {
    entries
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                HeaderValue::from_str(value).expect("header value"),
            )
        })
        .collect()
}

pub(super) fn object(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(super) fn image_item(url: &str) -> BTreeMap<String, GuardrailValue> {
    object(&[
        ("type", text("image_url")),
        (
            "image_url",
            GuardrailValue::Object(object(&[("url", text(url))])),
        ),
    ])
}

pub(super) fn data_url(mime_type: &str, payload: &str) -> String {
    format!("data:{mime_type};base64,{}", STANDARD.encode(payload))
}

pub(super) fn body_text(request: &Seen) -> String {
    str::from_utf8(&request.body)
        .expect("utf-8 body")
        .to_owned()
}

#[derive(Clone, Default)]
pub(super) struct Capture(pub(super) Arc<Mutex<Vec<u8>>>);

impl Capture {
    pub(super) fn text(&self) -> String {
        str::from_utf8(&self.0.lock().expect("capture"))
            .expect("utf-8 log")
            .to_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("log capture lock poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Capture {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}
