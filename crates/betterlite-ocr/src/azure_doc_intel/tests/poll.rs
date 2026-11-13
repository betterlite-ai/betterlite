use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::super::AzureDocIntelligenceOcrConfig;
use super::{NoHooks, params, plain_client, prepared_request, settings};
use crate::json::{Json, Object};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{OCR_RESPONSE_MAX_BYTES, PreparedOcrRequest, Secrets};
use crate::ocr_error::{OcrError, OcrTransportError};
use crate::ocr_http::{CallHooks, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::response::OCRResponse;

const RETRY_SECONDS_HEADER: &str = "retry-after";

struct ScriptedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct ScriptedServer {
    base: String,
    tasks: JoinSet<Vec<String>>,
}

impl ScriptedServer {
    async fn start(script: Vec<ScriptedResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the scripted server binds");
        let address = listener
            .local_addr()
            .expect("the scripted server has an address");
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, script));
        Self {
            base: format!("http://{address}"),
            tasks,
        }
    }

    async fn requests(mut self) -> Vec<String> {
        timeout(Duration::from_secs(10), self.tasks.join_next())
            .await
            .expect("the scripted server saw every request")
            .expect("the scripted server task runs")
            .expect("the scripted server task does not panic")
    }
}

async fn serve(listener: TcpListener, script: Vec<ScriptedResponse>) -> Vec<String> {
    let mut seen = Vec::new();
    for response in script {
        let (mut socket, _) = listener.accept().await.expect("the server accepts");
        let request = read_request(&mut socket).await;
        let rendered = response.render(&request);
        seen.push(request);
        socket
            .write_all(&rendered)
            .await
            .expect("the server writes the reply");
        socket.shutdown().await.expect("the server closes");
    }
    seen
}

fn finished(status: &str) -> ScriptedResponse {
    ScriptedResponse {
        status: 200,
        headers: Vec::new(),
        body: format!(r#"{{"status":"{status}"}}"#).into_bytes(),
    }
}

fn running_with_retry(retry_after: &str) -> ScriptedResponse {
    ScriptedResponse {
        status: 200,
        headers: vec![(RETRY_SECONDS_HEADER.to_owned(), retry_after.to_owned())],
        body: br#"{"status":"running"}"#.to_vec(),
    }
}

fn operation(body: &str) -> ScriptedResponse {
    ScriptedResponse {
        status: 200,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn accepted(retry_after: &str) -> ScriptedResponse {
    ScriptedResponse {
        status: 202,
        headers: vec![
            (
                "operation-location".to_owned(),
                "$origin/operations/abc?api-version=2024-11-30".to_owned(),
            ),
            (RETRY_SECONDS_HEADER.to_owned(), retry_after.to_owned()),
        ],
        body: br#"{"status":"running"}"#.to_vec(),
    }
}

fn succeeded(content: &str) -> String {
    format!(
        r#"{{"status":"succeeded","analyzeResult":{{"content":"{content}","pages":[{{"width":8.5,"height":11,"unit":"inch","lines":[{{"content":"hello"}}]}}]}}}}"#
    )
}

impl ScriptedResponse {
    fn render(&self, request: &str) -> Vec<u8> {
        let origin = origin_of(request);
        let host = origin.trim_start_matches("http://").to_owned();
        let mut head = format!(
            "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.body.len()
        );
        for (name, pattern) in &self.headers {
            let value = pattern.replace("$origin", &origin).replace("$host", &host);
            write!(head, "{name}: {value}\r\n").expect("the scripted header writes");
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

fn origin_of(request: &str) -> String {
    let host = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host")
                .then(|| value.trim().to_owned())
        })
        .expect("the request carries its host");
    format!("http://{host}")
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut buffer = read_head(socket).await;
    let head_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("the request has a header end")
        .saturating_add(4);
    let head = String::from_utf8(buffer.get(..head_end).unwrap_or_default().to_vec())
        .expect("the request head is UTF-8");
    let length = declared_length(&head);
    while buffer.len().saturating_sub(head_end) < length {
        let mut body_chunk = [0_u8; 2048];
        let read = socket
            .read(&mut body_chunk)
            .await
            .expect("the server reads the body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(body_chunk.get(..read).expect("the read fits"));
    }
    String::from_utf8(buffer).expect("the request is UTF-8")
}

async fn read_head(socket: &mut TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0_u8; 2048];
        let read = socket
            .read(&mut chunk)
            .await
            .expect("the server reads the request");
        buffer.extend_from_slice(chunk.get(..read).expect("the read fits"));
        if read == 0 || buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return buffer;
        }
    }
}

fn declared_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0)
}

fn connection(base: &str, extra: Vec<(&str, &str)>, options: OcrSettings) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: Some(OcrSourced {
                value: SecretString::from("request-key"),
                source: OcrInputSource::Request,
            }),
            api_base: Some(OcrSourced {
                value: base.to_owned(),
                source: OcrInputSource::Request,
            }),
        },
        OcrTransportConfig {
            extra_headers: extra
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            extra_headers_source: OcrInputSource::Request,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        options,
        Secrets::new(),
    )
}

fn request_for(
    base: &str,
    options: ScriptedSettings,
    extra: Vec<(&str, &str)>,
) -> PreparedOcrRequest {
    prepared_request(connection(base, extra, options.settings), options.params)
}

struct ScriptedSettings {
    settings: OcrSettings,
    params: Object,
}

impl ScriptedSettings {
    fn new(poll_timeout: Duration, options: &str) -> Self {
        Self {
            settings: settings(poll_timeout, 96, "2024-11-30"),
            params: params(options),
        }
    }
}

async fn call(
    base: &str,
    settings: ScriptedSettings,
    extra: Vec<(&str, &str)>,
) -> Result<OCRResponse, OcrError> {
    ocr(
        &AzureDocIntelligenceOcrConfig,
        &plain_client(),
        &request_for(base, settings, extra),
        &NoHooks,
    )
    .await
}

#[tokio::test]
async fn an_accepted_answer_polls_with_only_the_credential_headers() {
    let server = ScriptedServer::start(vec![
        accepted("1"),
        finished("running"),
        operation(&succeeded("final")),
    ])
    .await;
    let response = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        vec![("Authorization", "Bearer supplied"), ("x-caller", "yes")],
    )
    .await
    .expect("the operation finishes");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 3);
    let analyze = requests[0].to_ascii_lowercase();
    assert!(
        analyze.starts_with(
            "post /documentintelligence/documentmodels/prebuilt-layout:analyze?api-version=2024-11-30 http/1.1"
        ),
        "{}",
        requests[0]
    );
    assert!(analyze.contains("authorization: bearer supplied"));
    assert!(analyze.contains("x-caller: yes"));
    assert!(analyze.contains(r#""urlsource":"https://example.test/a.pdf""#));
    let first_poll = requests[1].to_ascii_lowercase();
    assert!(
        first_poll.starts_with("get /operations/abc?api-version=2024-11-30 http/1.1"),
        "{}",
        requests[1]
    );
    assert!(first_poll.contains("authorization: bearer supplied"));
    assert!(!first_poll.contains("x-caller"));
    assert!(requests[2].starts_with("GET /operations/abc?api-version=2024-11-30"));
    assert_eq!(response.content.as_deref(), Some("final"));
    assert_eq!(response.pages.len(), 1);
}

#[tokio::test]
async fn the_subscription_key_header_is_kept_and_others_are_dropped() {
    let server = ScriptedServer::start(vec![accepted("1"), operation(&succeeded("final"))]).await;
    call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect("the operation finishes");
    let requests = server.requests().await;
    let poll = requests[1].to_ascii_lowercase();
    assert!(
        poll.contains("ocp-apim-subscription-key: request-key"),
        "{poll}"
    );
    assert!(!poll.contains("content-type"));
    assert!(!poll.contains("x-caller"));
}

#[tokio::test]
async fn native_format_keeps_the_final_operation_not_the_accepted_body() {
    let mut accepted_body = accepted("1");
    accepted_body.body = br#"{"status":"running","analyzeResult":{"content":"accepted"}}"#.to_vec();
    let server = ScriptedServer::start(vec![accepted_body, operation(&succeeded("final"))]).await;
    let response = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), r#"{"req_format":"native"}"#),
        Vec::new(),
    )
    .await
    .expect("the operation finishes");
    let Some(Json::Object(native)) = response.provider_native_response else {
        panic!("the operation is kept natively");
    };
    let Some(Json::Object(result)) = native.get("analyzeResult") else {
        panic!("the native operation carries its result");
    };
    assert_eq!(result.get("content"), Some(&Json::Text("final".to_owned())));
    server.requests().await;
}

#[tokio::test]
async fn a_direct_finished_answer_needs_no_poll() {
    let server = ScriptedServer::start(vec![operation(&succeeded("direct"))]).await;
    let response = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect("the operation finishes");
    assert_eq!(response.content.as_deref(), Some("direct"));
    assert_eq!(response.model, "azure_ai/doc-intelligence/prebuilt-layout");
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn the_async_path_uses_the_configured_dpi() {
    let server = ScriptedServer::start(vec![operation(&succeeded("direct"))]).await;
    let scripted = ScriptedSettings {
        settings: settings(Duration::from_secs(120), 72, "2024-11-30"),
        params: params("{}"),
    };
    let response = call(&server.base, scripted, Vec::new())
        .await
        .expect("the operation finishes");
    let dimensions = response.pages[0].dimensions.as_ref().unwrap();
    assert_eq!(dimensions.width, Some(612));
    assert_eq!(dimensions.dpi, Some(72));
}

#[tokio::test]
async fn a_direct_running_answer_is_an_operation_status() {
    let server = ScriptedServer::start(vec![finished("running")]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("a running direct answer is not finished");
    assert!(matches!(error, OcrError::OperationStatus(status) if status == "running"));
}

#[tokio::test]
async fn a_failed_poll_is_an_operation_status() {
    let server = ScriptedServer::start(vec![accepted("1"), finished("failed")]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("a failed operation is not finished");
    assert!(matches!(error, OcrError::OperationStatus(status) if status == "failed"));
}

#[tokio::test]
async fn a_missing_operation_location_is_poll_location() {
    let mut response = accepted("1");
    response.headers.clear();
    let server = ScriptedServer::start(vec![response]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the location is missing");
    assert!(matches!(error, OcrError::PollLocation));
}

#[tokio::test]
async fn a_foreign_operation_location_is_poll_origin() {
    let mut response = accepted("1");
    response.headers[0] = (
        "operation-location".to_owned(),
        "http://other.example/operations/abc".to_owned(),
    );
    let server = ScriptedServer::start(vec![response]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the origin differs");
    assert!(matches!(error, OcrError::PollOrigin));
}

#[rstest]
#[case::user_and_password("http://user:pass@$host/operations/abc")]
#[case::user_only("http://user@$host/operations/abc")]
#[case::password_only("http://:pass@$host/operations/abc")]
#[tokio::test]
async fn an_operation_location_with_credentials_is_poll_origin(#[case] location: &str) {
    let mut response = accepted("1");
    response.headers[0] = ("operation-location".to_owned(), location.to_owned());
    let server = ScriptedServer::start(vec![response]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the location carries credentials");
    assert!(matches!(error, OcrError::PollOrigin));
}

#[tokio::test]
async fn a_redirecting_poll_is_not_followed() {
    let mut redirect = ScriptedResponse {
        status: 302,
        headers: vec![("location".to_owned(), "$origin/operations/moved".to_owned())],
        body: Vec::new(),
    };
    redirect
        .headers
        .push(("content-type".to_owned(), "text/plain".to_owned()));
    let server = ScriptedServer::start(vec![accepted("1"), redirect]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the poll answer is not a finished operation");
    assert!(
        matches!(
            error,
            OcrError::Transport(OcrTransportError::Http { status: 302, .. })
        ),
        "{error:?}"
    );
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn a_poll_timeout_without_time_left_stops_before_the_first_poll() {
    let server = ScriptedServer::start(vec![accepted("1")]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::ZERO, "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the deadline is already gone");
    assert!(matches!(error, OcrError::PollTimeout));
    assert_eq!(server.requests().await.len(), 1);
}

#[tokio::test]
async fn a_poll_timeout_caps_the_retry_wait() {
    let server = ScriptedServer::start(vec![accepted("5"), finished("running")]).await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(1), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the retry wait outlives the deadline");
    assert!(matches!(error, OcrError::PollTimeout));
    server.requests().await;
}

#[tokio::test]
async fn a_retry_after_below_one_second_waits_the_minimum() {
    let server = ScriptedServer::start(vec![
        accepted("1"),
        running_with_retry("0"),
        operation(&succeeded("final")),
    ])
    .await;
    let started = Instant::now();
    call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect("the operation finishes");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(900),
        "the wait was {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1900),
        "the wait used the default instead of Retry-After: {elapsed:?}"
    );
    server.requests().await;
}

struct RecordingHooks {
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait]
impl CallHooks for RecordingHooks {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError> {
        self.bodies
            .lock()
            .expect("the hook record locks")
            .push(body.to_vec());
        Ok(())
    }
}

#[tokio::test]
async fn hooks_see_the_accepted_body_and_the_final_operation() {
    let mut accepted_body = accepted("1");
    accepted_body.body = br#"{"status":"notStarted"}"#.to_vec();
    let final_body = succeeded("final");
    let server = ScriptedServer::start(vec![
        accepted_body,
        finished("running"),
        operation(&final_body),
    ])
    .await;
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let hooks = RecordingHooks {
        bodies: Arc::clone(&bodies),
    };
    ocr(
        &AzureDocIntelligenceOcrConfig,
        &plain_client(),
        &request_for(
            &server.base,
            ScriptedSettings::new(Duration::from_secs(10), "{}"),
            Vec::new(),
        ),
        &hooks,
    )
    .await
    .expect("the operation finishes");
    let recorded = bodies.lock().expect("the hook record locks").clone();
    assert_eq!(
        recorded,
        vec![
            br#"{"status":"notStarted"}"#.to_vec(),
            final_body.into_bytes()
        ]
    );
    server.requests().await;
}

#[tokio::test]
async fn a_poll_error_status_is_a_transport_http_error() {
    let server = ScriptedServer::start(vec![
        accepted("1"),
        ScriptedResponse {
            status: 500,
            headers: Vec::new(),
            body: b"broken".to_vec(),
        },
    ])
    .await;
    let error = call(
        &server.base,
        ScriptedSettings::new(Duration::from_secs(10), "{}"),
        Vec::new(),
    )
    .await
    .expect_err("the poll answer is an error");
    assert!(
        matches!(
            error,
            OcrError::Transport(OcrTransportError::Http { status: 500, .. })
        ),
        "{error:?}"
    );
    server.requests().await;
}
