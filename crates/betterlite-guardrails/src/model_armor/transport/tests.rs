#![cfg(test)]

use std::future::{Future, IntoFuture as _};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode, Uri};
use betterlite_http::proxy::limits::ReadError;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::error::{ArmorCallError, ArmorTokenError, TransportError};
use super::project::ResolvedProject;
use super::tokens::ArmorTokens;
use super::wire::Source;
use super::wire::{SanitizeBody, TextPrompt};
use super::{MAX_RESPONSE_BYTES, ModelArmorTransport, file_body, text_body};
use crate::model_armor::decisions::should_block_content;
use crate::model_armor::file_scanning::ByteDataType;

type Server = JoinSet<Result<(), io::Error>>;

#[derive(Debug, Default, Clone)]
struct Captured {
    requests: usize,
    path: String,
    authorization: String,
    content_type: String,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn ok(body: &str) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.as_bytes().to_vec(),
        }
    }
}

#[derive(Clone)]
struct StubTokens {
    calls: Arc<Mutex<usize>>,
    project: Option<String>,
}

impl StubTokens {
    fn new(project: Option<&str>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(0)),
            project: project.map(str::to_owned),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("token calls")
    }
}

#[async_trait]
impl ArmorTokens for StubTokens {
    type Error = ArmorTokenError;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError> {
        let mut calls = self.calls.lock().expect("token calls");
        *calls = (*calls).saturating_add(1);
        Ok((SecretString::from("secret-token"), self.project.clone()))
    }
}

async fn serve(reply: Reply) -> (String, Arc<Mutex<Captured>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let captured = Arc::new(Mutex::new(Captured::default()));
    let handler_captured = Arc::clone(&captured);
    let handler = move |headers: HeaderMap, uri: Uri, body: Bytes| {
        let request_capture = Arc::clone(&handler_captured);
        let reply = reply.clone();
        async move {
            let mut request = request_capture.lock().expect("captured request");
            request.requests = request.requests.saturating_add(1);
            request.path = uri.path().to_owned();
            request.authorization = header(&headers, AUTHORIZATION.as_str());
            request.content_type = header(&headers, CONTENT_TYPE.as_str());
            request.body = body.to_vec();
            drop(request);
            (reply.status, Body::from(reply.body))
        }
    };
    let router = Router::new().fallback(handler);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (endpoint, captured, server)
}

async fn hang() -> (String, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let answer = || async {
        sleep(Duration::from_secs(30)).await;
        StatusCode::OK
    };
    let router = Router::new().fallback(answer);
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (endpoint, server)
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn transport(endpoint: Option<&str>, tokens: StubTokens) -> ModelArmorTransport<StubTokens> {
    ModelArmorTransport {
        template_id: Some("tmpl".to_owned()),
        project_id: Some("proj".to_owned()),
        location: "europe-west4".to_owned(),
        api_endpoint: endpoint.map(str::to_owned),
        sanitize_error_detail: true,
        timeout: None,
        client: reqwest::Client::default(),
        tokens: Arc::new(tokens),
        resolved_project: ResolvedProject::default(),
    }
}

fn offline(tokens: StubTokens) -> ModelArmorTransport<StubTokens> {
    transport(Some("http://127.0.0.1:1"), tokens)
}

#[derive(Clone, Default)]
struct LogSink(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogSink {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn logged<Run>(run: Run) -> String
where
    Run: Future<Output = ()>,
{
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    run.await;
    drop(guard);
    String::from_utf8(sink.0.lock().expect("log").clone()).expect("utf8 log")
}

#[test]
fn url_default_endpoint_for_both_sources() {
    let transport = transport(None, StubTokens::new(None));
    assert_eq!(
        transport.api_endpoint(),
        "https://modelarmor.europe-west4.rep.googleapis.com"
    );
    assert_eq!(
        transport.sanitize_url(Source::UserPrompt),
        "https://modelarmor.europe-west4.rep.googleapis.com/v1/projects/proj/locations/europe-west4/templates/tmpl:sanitizeUserPrompt"
    );
    assert_eq!(
        transport.sanitize_url(Source::ModelResponse),
        "https://modelarmor.europe-west4.rep.googleapis.com/v1/projects/proj/locations/europe-west4/templates/tmpl:sanitizeModelResponse"
    );
}

#[test]
fn url_configured_endpoint_is_used_as_given() {
    let transport = transport(Some("https://proxy.test:8443/base/"), StubTokens::new(None));
    assert_eq!(transport.api_endpoint(), "https://proxy.test:8443/base/");
    assert_eq!(
        transport.sanitize_url(Source::UserPrompt),
        "https://proxy.test:8443/base//v1/projects/proj/locations/europe-west4/templates/tmpl:sanitizeUserPrompt"
    );
}

#[test]
fn url_empty_endpoint_falls_back_to_the_default() {
    let transport = transport(Some(""), StubTokens::new(None));
    assert_eq!(
        transport.api_endpoint(),
        "https://modelarmor.europe-west4.rep.googleapis.com"
    );
}

#[test]
fn url_missing_project_leaves_an_empty_segment() {
    let mut transport = transport(None, StubTokens::new(None));
    transport.project_id = None;
    assert_eq!(
        transport.sanitize_url(Source::UserPrompt),
        "https://modelarmor.europe-west4.rep.googleapis.com/v1/projects//locations/europe-west4/templates/tmpl:sanitizeUserPrompt"
    );
}

#[tokio::test]
async fn url_resolved_project_fills_in_only_when_none_is_configured() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(Some("resolved-project")));
    transport.project_id = None;
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(result.is_ok());
    assert!(
        captured
            .lock()
            .expect("captured")
            .path
            .contains("/projects/resolved-project/")
    );
    assert!(
        transport
            .sanitize_url(Source::UserPrompt)
            .contains("/projects/resolved-project/")
    );
}

#[tokio::test]
async fn url_configured_project_is_never_replaced() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(Some("resolved-project")));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(result.is_ok());
    let path = captured.lock().expect("captured").path.clone();
    assert!(path.contains("/projects/proj/"));
    assert!(!path.contains("resolved-project"));
}

#[tokio::test]
async fn url_empty_configured_project_takes_the_resolved_one() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(Some("resolved-project")));
    transport.project_id = Some(String::new());
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(result.is_ok());
    assert!(
        captured
            .lock()
            .expect("captured")
            .path
            .contains("/projects/resolved-project/")
    );
}

#[test]
fn url_empty_resolved_project_is_not_remembered() {
    let transport = transport(None, StubTokens::new(None));
    transport.resolved_project.remember(None, Some(""));
    transport
        .resolved_project
        .remember(None, Some("resolved-later"));
    assert_eq!(
        transport.resolved_project.effective(None),
        Some("resolved-later".to_owned())
    );
}

#[test]
fn body_text_for_both_sources() {
    assert_eq!(
        serde_json::to_string(&text_body("hello", Source::UserPrompt)).expect("body"),
        r#"{"userPromptData":{"text":"hello"}}"#
    );
    assert_eq!(
        serde_json::to_string(&text_body("hello", Source::ModelResponse)).expect("body"),
        r#"{"modelResponseData":{"text":"hello"}}"#
    );
}

#[test]
fn body_empty_text_is_a_present_text_field() {
    assert_eq!(
        serde_json::to_string(&text_body("", Source::UserPrompt)).expect("body"),
        r#"{"userPromptData":{"text":""}}"#
    );
}

#[test]
fn body_file_for_both_sources() {
    assert_eq!(
        serde_json::to_string(&file_body(b"hello", ByteDataType::Pdf, Source::UserPrompt))
            .expect("body"),
        r#"{"userPromptData":{"byteItem":{"byteDataType":"PDF","byteData":"aGVsbG8="}}}"#
    );
    assert_eq!(
        serde_json::to_string(&file_body(
            b"hello",
            ByteDataType::WordDocument,
            Source::ModelResponse
        ))
        .expect("body"),
        r#"{"modelResponseData":{"byteItem":{"byteDataType":"WORD_DOCUMENT","byteData":"aGVsbG8="}}}"#
    );
}

#[test]
fn body_file_uses_standard_padded_base64() {
    assert_eq!(
        serde_json::to_string(&file_body(
            &[0xff, 0xfe],
            ByteDataType::Csv,
            Source::UserPrompt
        ))
        .expect("body"),
        r#"{"userPromptData":{"byteItem":{"byteDataType":"CSV","byteData":"//4="}}}"#
    );
}

#[tokio::test]
async fn request_posts_json_with_the_bearer_token_and_content_type() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let tokens = StubTokens::new(None);
    let transport = transport(Some(&endpoint), tokens.clone());
    let _response = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await
        .expect("request");
    assert_eq!(tokens.calls(), 1);
    let captured = captured.lock().expect("captured");
    assert_eq!(captured.requests, 1);
    assert_eq!(
        captured.path,
        "/v1/projects/proj/locations/europe-west4/templates/tmpl:sanitizeUserPrompt"
    );
    assert_eq!(captured.authorization, "Bearer secret-token");
    assert_eq!(captured.content_type, "application/json");
    let sent: SanitizeBody = serde_json::from_slice(&captured.body).expect("body");
    assert_eq!(sent, text_body("hello", Source::UserPrompt));
}

#[tokio::test]
async fn request_returns_the_decoded_verdict_and_body() {
    let (endpoint, _captured, _server) = serve(Reply::ok(
        r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND"}}}}}}"#,
    ))
    .await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await
        .expect("request");
    assert!(should_block_content(&result.verdict, false));
    assert!(!should_block_content(&result.verdict, true));
}

#[tokio::test]
async fn request_reads_a_body_above_two_kib() {
    let padding = "a".repeat(4096);
    let body =
        format!(r#"{{"sanitizationResult":{{"filterResults":{{}}}},"padding":"{padding}"}}"#);
    let (endpoint, _captured, _server) = serve(Reply::ok(&body)).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn request_uses_the_file_body_over_the_content() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let file_bytes = b"hello".to_vec();
    let result = transport
        .make_model_armor_request(
            Some("ignored"),
            Source::UserPrompt,
            Some((file_bytes.as_slice(), ByteDataType::Pdf)),
        )
        .await;
    assert!(result.is_ok());
    let sent: SanitizeBody =
        serde_json::from_slice(&captured.lock().expect("captured").body).expect("body");
    assert_eq!(
        sent,
        file_body(file_bytes.as_slice(), ByteDataType::Pdf, Source::UserPrompt)
    );
}

#[tokio::test]
async fn request_model_response_source_uses_the_model_response_method() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::ModelResponse, None)
        .await;
    assert!(result.is_ok());
    assert!(
        captured
            .lock()
            .expect("captured")
            .path
            .ends_with(":sanitizeModelResponse")
    );
}

#[tokio::test]
async fn request_without_a_template_fails_before_the_token_and_the_request() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let tokens = StubTokens::new(None);
    let mut transport = transport(Some(&endpoint), tokens.clone());
    transport.template_id = None;
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::MissingTemplate))
    ));
    assert_eq!(tokens.calls(), 0);
    assert_eq!(captured.lock().expect("captured").requests, 0);
}

#[tokio::test]
async fn request_without_content_or_file_fails_before_the_request() {
    let (endpoint, captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(None, Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::NoContent))
    ));
    assert_eq!(captured.lock().expect("captured").requests, 0);
}

#[tokio::test]
async fn request_timeout_is_a_typed_timeout() {
    let (endpoint, _server) = hang().await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(None));
    transport.timeout = Some(Duration::from_millis(50));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::Timeout(_)))
    ));
}

#[tokio::test]
async fn error_non_2xx_is_an_api_error_with_the_sanitized_detail() {
    let mut reply = Reply::ok(r#"{"error":{"message":"bad template"}}"#);
    reply.status = StatusCode::BAD_REQUEST;
    let (endpoint, _captured, _server) = serve(reply).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    match result {
        Err(ArmorCallError::Api(error)) => {
            assert_eq!(error.detail, "Model Armor API error (upstream 400)");
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn error_full_detail_when_sanitize_error_detail_is_false() {
    let mut reply = Reply::ok(r#"{"error":{"message":"bad template"}}"#);
    reply.status = StatusCode::INTERNAL_SERVER_ERROR;
    let (endpoint, _captured, _server) = serve(reply).await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(None));
    transport.sanitize_error_detail = false;
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    match result {
        Err(ArmorCallError::Api(error)) => {
            assert_eq!(
                error.detail,
                r#"Model Armor API error (upstream 500): {"error":{"message":"bad template"}}"#
            );
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn error_2xx_non_200_is_an_api_error() {
    let (endpoint, _captured, _server) = serve(Reply {
        status: StatusCode::NO_CONTENT,
        body: Vec::new(),
    })
    .await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    match result {
        Err(ArmorCallError::Api(error)) => {
            assert_eq!(error.detail, "Model Armor API error (upstream 204)");
        }
        other => panic!("expected an API error, got {other:?}"),
    }
}

#[tokio::test]
async fn error_invalid_json_is_a_response_json_error() {
    let (endpoint, _captured, _server) = serve(Reply::ok("not json")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::ResponseJson(_)))
    ));
}

#[tokio::test]
async fn error_non_utf8_body_is_a_response_text_error() {
    let (endpoint, _captured, _server) = serve(Reply {
        status: StatusCode::OK,
        body: vec![0xff, 0xfe],
    })
    .await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::ResponseText(_)))
    ));
}

#[tokio::test]
async fn error_response_above_one_mib_is_a_body_error() {
    let body = "a".repeat(MAX_RESPONSE_BYTES + 1);
    let (endpoint, _captured, _server) = serve(Reply::ok(&body)).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::Body(
            ReadError::TooLarge { limit }
        ))) if limit == MAX_RESPONSE_BYTES
    ));
}

#[tokio::test]
async fn error_connection_failure_is_a_request_error() {
    let transport = offline(StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(
        result,
        Err(ArmorCallError::Transport(TransportError::Request(_)))
    ));
}

#[tokio::test]
async fn log_file_request_never_prints_the_bytes_or_the_token() {
    let (endpoint, _captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let file_bytes = b"secret-bytes".to_vec();
    let log = logged(async {
        let result = transport
            .make_model_armor_request(
                None,
                Source::UserPrompt,
                Some((&file_bytes, ByteDataType::Pdf)),
            )
            .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains(&endpoint));
    assert!(log.contains("PDF"));
    assert!(log.contains("bytes"));
    assert!(!log.contains("c2VjcmV0LWJ5dGVz"));
    assert!(!log.contains("secret-token"));
}

#[tokio::test]
async fn log_text_request_shows_only_the_url_when_sanitized() {
    let (endpoint, _captured, _server) = serve(Reply::ok("{}")).await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let log = logged(async {
        let result = transport
            .make_model_armor_request(Some("top-secret-prompt"), Source::UserPrompt, None)
            .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains(&endpoint));
    assert!(!log.contains("top-secret-prompt"));
    assert!(!log.contains("secret-token"));
}

#[tokio::test]
async fn log_text_request_shows_the_body_when_not_sanitized() {
    let (endpoint, _captured, _server) = serve(Reply::ok("{}")).await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(None));
    transport.sanitize_error_detail = false;
    let log = logged(async {
        let result = transport
            .make_model_armor_request(Some("top-secret-prompt"), Source::UserPrompt, None)
            .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains("top-secret-prompt"));
}

#[tokio::test]
async fn log_response_shows_only_the_status_when_sanitized() {
    let (endpoint, _captured, _server) = serve(Reply::ok(
        r#"{"sanitizationResult":{"response-marker":true}}"#,
    ))
    .await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let log = logged(async {
        let result = transport
            .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
            .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains("200"));
    assert!(!log.contains("response-marker"));
}

#[tokio::test]
async fn log_response_shows_the_body_when_not_sanitized() {
    let (endpoint, _captured, _server) = serve(Reply::ok(
        r#"{"sanitizationResult":{"response-marker":true}}"#,
    ))
    .await;
    let mut transport = transport(Some(&endpoint), StubTokens::new(None));
    transport.sanitize_error_detail = false;
    let log = logged(async {
        let result = transport
            .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
            .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains("response-marker"));
}

#[tokio::test]
async fn error_api_errors_are_typed_before_other_transport_errors() {
    let (endpoint, _captured, _server) = serve(Reply {
        status: StatusCode::TOO_MANY_REQUESTS,
        body: b"slow down".to_vec(),
    })
    .await;
    let transport = transport(Some(&endpoint), StubTokens::new(None));
    let result = transport
        .make_model_armor_request(Some("hello"), Source::UserPrompt, None)
        .await;
    assert!(matches!(result, Err(ArmorCallError::Api(_))));
}

#[test]
fn body_prompt_data_is_untagged_over_the_text_and_byte_items() {
    let text: super::wire::PromptData = serde_json::from_str(r#"{"text":"hello"}"#).expect("text");
    assert_eq!(
        text,
        super::wire::PromptData::Text(TextPrompt {
            text: "hello".to_owned()
        })
    );
    let bytes: super::wire::PromptData =
        serde_json::from_str(r#"{"byteItem":{"byteDataType":"TXT","byteData":""}}"#)
            .expect("bytes");
    assert!(matches!(bytes, super::wire::PromptData::Bytes(_)));
}
