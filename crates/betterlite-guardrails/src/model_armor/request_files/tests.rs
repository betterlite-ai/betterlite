#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, IntoFuture as _};
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, StatusCode, Uri};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::metadata::{MetadataMap, MetadataSlots};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{FileScanSettings, ScanError, scan_request_files};
use crate::model_armor::file_scanning::MAX_FILE_SIZE_BYTES;
use crate::model_armor::transport::ModelArmorTransport;
use crate::model_armor::transport::error::{ArmorTokenError, TransportError};
use crate::model_armor::transport::project::ResolvedProject;
use crate::model_armor::transport::tokens::ArmorTokens;

const CLEAN: &str = r#"{"sanitizationResult":{"filterResults":{}}}"#;
const BLOCKED_PII: &str = r#"{"sanitizationResult":{"filterResults":{"sdp":{"sdpFilterResult":{"deidentifyResult":{"matchState":"MATCH_FOUND"}}}}}}"#;
const STATUS_KEY: &str = "_model_armor_status";
const RESPONSE_KEY: &str = "_model_armor_response";
const UNSCANNABLE: &str = "Model Armor could not scan an attachment and blocked the request: 1 attachment(s) reference a document with no inline bytes (file_id or remote URL) that Model Armor cannot scan";

type Server = JoinSet<Result<(), io::Error>>;

struct StubTokens;

#[async_trait]
impl ArmorTokens for StubTokens {
    type Error = ArmorTokenError;

    async fn access_token(&self) -> Result<(SecretString, Option<String>), ArmorTokenError> {
        Ok((SecretString::from("token"), None))
    }
}

async fn serve(replies: Vec<(StatusCode, String)>) -> (String, Arc<Mutex<usize>>, Server) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let endpoint = format!("http://{}", listener.local_addr().expect("address"));
    let requests = Arc::new(Mutex::new(0));
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let handler_requests = Arc::clone(&requests);
    let handler_queue = Arc::clone(&queue);
    let handler = move |_headers: HeaderMap, _uri: Uri, _body: Bytes| {
        let request_count = Arc::clone(&handler_requests);
        let replies = Arc::clone(&handler_queue);
        async move {
            let mut total = request_count.lock().expect("requests");
            *total = total.saturating_add(1);
            drop(total);
            let reply = replies.lock().expect("replies").pop_front();
            let (status, body) = reply.unwrap_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "no reply queued".to_owned(),
                )
            });
            (status, Body::from(body))
        }
    };
    let router = Router::new()
        .fallback(handler)
        .layer(DefaultBodyLimit::disable());
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (endpoint, requests, server)
}

fn transport(endpoint: &str, sanitize_error_detail: bool) -> ModelArmorTransport<StubTokens> {
    ModelArmorTransport {
        template_id: Some("tmpl".to_owned()),
        project_id: Some("proj".to_owned()),
        location: "us-central1".to_owned(),
        api_endpoint: Some(endpoint.to_owned()),
        sanitize_error_detail,
        timeout: None,
        client: reqwest::Client::default(),
        tokens: Arc::new(StubTokens),
        resolved_project: ResolvedProject::default(),
    }
}

fn messages(block: &str) -> Vec<Message> {
    let raw = format!(r#"[{{"role":"user","content":[{block}]}}]"#);
    serde_json::from_str(&raw).expect("messages")
}

fn messages_pair(first: &str, second: &str) -> Vec<Message> {
    let raw = format!(r#"[{{"role":"user","content":[{first},{second}]}}]"#);
    serde_json::from_str(&raw).expect("messages")
}

fn pdf_attachment() -> String {
    r#"{"type":"file","file":{"file_data":"aGVsbG8=","format":"application/pdf"}}"#.to_owned()
}

fn slots() -> MetadataSlots {
    MetadataSlots {
        metadata: Some(MetadataMap::new()),
        litellm_metadata: None,
    }
}

fn settings(name: Option<&str>, fail_on_error: bool, skip: bool) -> FileScanSettings {
    FileScanSettings {
        guardrail_name: name.map(str::to_owned),
        fail_on_error,
        skip_unscannable_attachments: skip,
    }
}

fn blocked(error: ScanError<ArmorTokenError>) -> (u16, IndexMap<String, GuardrailValue>) {
    match error {
        ScanError::Blocked { status, detail } => (status, detail),
        other => panic!("expected a block, got {other:?}"),
    }
}

fn metadata(request_data: &MetadataSlots) -> &MetadataMap {
    request_data.metadata.as_ref().expect("metadata bucket")
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

#[tokio::test]
async fn request_files_returns_before_touching_the_request_data() {
    let (endpoint, requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let mut request_data = MetadataSlots {
        metadata: None,
        litellm_metadata: None,
    };
    let result = scan_request_files(
        &messages(r#"{"type":"text","text":"hi"}"#),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(result.is_ok());
    assert!(request_data.metadata.is_none());
    assert!(request_data.litellm_metadata.is_none());
    assert_eq!(*requests.lock().expect("requests"), 0);
}

#[tokio::test]
async fn request_files_records_the_guardrail_name_once() {
    let (endpoint, requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    request_data.metadata = Some(MetadataMap::from([(
        "applied_guardrails".to_owned(),
        Json::List(vec![Json::Text("armor".to_owned())]),
    )]));
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        metadata(&request_data).get("applied_guardrails"),
        Some(&Json::List(vec![Json::Text("armor".to_owned())]))
    );
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("success".to_owned()))
    );
    assert!(metadata(&request_data).contains_key(RESPONSE_KEY));
    assert_eq!(*requests.lock().expect("requests"), 1);
}

#[tokio::test]
async fn request_files_replaces_a_non_list_applied_guardrails_value() {
    let (endpoint, _requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    request_data.metadata = Some(MetadataMap::from([(
        "applied_guardrails".to_owned(),
        Json::Text("not-a-list".to_owned()),
    )]));
    let log = logged(async {
        let result = scan_request_files(
            &messages(&pdf_attachment()),
            &mut request_data,
            &transport,
            &settings(Some("armor"), true, false),
        )
        .await;
        assert!(result.is_ok());
    })
    .await;
    assert_eq!(
        metadata(&request_data).get("applied_guardrails"),
        Some(&Json::List(vec![Json::Text("armor".to_owned())]))
    );
    assert!(log.contains("replacing an applied_guardrails value that is not a list"));
    assert!(log.contains("key=\"applied_guardrails\""));
    assert!(log.contains("found=\"string\""));
}

#[tokio::test]
async fn request_files_without_a_guardrail_name_still_creates_the_bucket() {
    let (endpoint, _requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(None, true, false),
    )
    .await;
    assert!(result.is_ok());
    assert!(!metadata(&request_data).contains_key("applied_guardrails"));
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("success".to_owned()))
    );
}

#[tokio::test]
async fn request_files_writes_to_the_litellm_metadata_bucket_when_present() {
    let (endpoint, _requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = MetadataSlots {
        metadata: Some(MetadataMap::from([(
            "caller".to_owned(),
            Json::Text("kept".to_owned()),
        )])),
        litellm_metadata: Some(MetadataMap::new()),
    };
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        request_data
            .litellm_metadata
            .as_ref()
            .expect("litellm metadata")
            .get(STATUS_KEY),
        Some(&Json::Text("success".to_owned()))
    );
    assert_eq!(request_data.metadata.as_ref().expect("metadata").len(), 1,);
}

#[tokio::test]
async fn request_files_unscannable_references_block_with_the_400_detail() {
    let (endpoint, requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(
            r#"{"type":"file","file":{"file_data":"gs://bucket/a.pdf","format":"application/pdf"}}"#,
        ),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    let (status, detail) = blocked(result.expect_err("blocked"));
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(UNSCANNABLE.to_owned()))
    );
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("blocked".to_owned()))
    );
    assert_eq!(
        metadata(&request_data).get("applied_guardrails"),
        Some(&Json::List(vec![Json::Text("armor".to_owned())]))
    );
    assert_eq!(*requests.lock().expect("requests"), 0);
}

#[tokio::test]
async fn request_files_unscannable_references_do_not_block_when_fail_open() {
    let (endpoint, _requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(r#"{"type":"file","file":{"file_data":"https://example.test/a.pdf"}}"#),
        &mut request_data,
        &transport,
        &settings(Some("armor"), false, false),
    )
    .await;
    assert!(result.is_ok());
    assert!(!metadata(&request_data).contains_key(STATUS_KEY));
}

#[tokio::test]
async fn request_files_skip_unscannable_attachments_returns_without_touching_metadata() {
    let (endpoint, requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let mut request_data = MetadataSlots {
        metadata: None,
        litellm_metadata: None,
    };
    let result = scan_request_files(
        &messages(r#"{"type":"document","source":{"media_type":"application/pdf"}}"#),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, true),
    )
    .await;
    assert!(result.is_ok());
    assert!(request_data.metadata.is_none());
    assert_eq!(*requests.lock().expect("requests"), 0);
}

#[tokio::test]
async fn request_files_oversize_attachments_block_before_the_request() {
    let (endpoint, requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let payload = STANDARD.encode(vec![b'a'; MAX_FILE_SIZE_BYTES.saturating_add(1)]);
    let block = format!(
        r#"{{"type":"file","file":{{"file_data":"data:application/pdf;base64,{payload}"}}}}"#
    );
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&block),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    let (status, detail) = blocked(result.expect_err("blocked"));
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Model Armor could not scan an attachment and blocked the request: attachment of 4194305 bytes exceeds Model Armor's 4194304 byte scan limit".to_owned()
        ))
    );
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("blocked".to_owned()))
    );
    assert_eq!(*requests.lock().expect("requests"), 0);
}

#[tokio::test]
async fn request_files_oversize_attachments_are_skipped_when_fail_open() {
    let (endpoint, requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let payload = STANDARD.encode(vec![b'a'; MAX_FILE_SIZE_BYTES.saturating_add(1)]);
    let block = format!(
        r#"{{"type":"file","file":{{"file_data":"data:application/pdf;base64,{payload}"}}}}"#
    );
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&block),
        &mut request_data,
        &transport,
        &settings(Some("armor"), false, false),
    )
    .await;
    assert!(result.is_ok());
    assert!(!metadata(&request_data).contains_key(STATUS_KEY));
    assert_eq!(*requests.lock().expect("requests"), 0);
}

#[tokio::test]
async fn request_files_api_errors_follow_fail_on_error() {
    let (endpoint, _requests, _server) = serve(vec![(
        StatusCode::BAD_REQUEST,
        r#"{"error":"bad"}"#.to_owned(),
    )])
    .await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    match result {
        Err(ScanError::Api(error)) => {
            assert_eq!(error.detail, "Model Armor API error (upstream 400)");
        }
        other => panic!("expected an API error, got {other:?}"),
    }
    assert!(!metadata(&request_data).contains_key(RESPONSE_KEY));
}

#[tokio::test]
async fn request_files_api_errors_are_skipped_when_fail_open() {
    let (endpoint, _requests, _server) = serve(vec![
        (StatusCode::BAD_REQUEST, r#"{"error":"bad"}"#.to_owned()),
        (StatusCode::OK, CLEAN.to_owned()),
    ])
    .await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages_pair(
            &pdf_attachment(),
            &pdf_attachment().replace("aGVsbG8=", "d29ybGQ="),
        ),
        &mut request_data,
        &transport,
        &settings(Some("armor"), false, false),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("success".to_owned()))
    );
}

#[tokio::test]
async fn request_files_transport_errors_follow_fail_on_error() {
    let transport = transport("http://127.0.0.1:1", true);
    let mut closed = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut closed,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(matches!(
        result,
        Err(ScanError::Transport(TransportError::Request(_)))
    ));
    let mut open = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut open,
        &transport,
        &settings(Some("armor"), false, false),
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn request_files_a_deidentify_match_blocks_with_400() {
    let (endpoint, _requests, _server) =
        serve(vec![(StatusCode::OK, BLOCKED_PII.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    let (status, detail) = blocked(result.expect_err("blocked"));
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked by Model Armor".to_owned()
        ))
    );
    assert!(!detail.contains_key("model_armor_response"));
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("blocked".to_owned()))
    );
    assert!(metadata(&request_data).contains_key(RESPONSE_KEY));
}

#[tokio::test]
async fn request_files_block_detail_carries_the_response_when_not_sanitized() {
    let (endpoint, _requests, _server) =
        serve(vec![(StatusCode::OK, BLOCKED_PII.to_owned())]).await;
    let transport = transport(&endpoint, false);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    let (_, detail) = blocked(result.expect_err("blocked"));
    assert!(detail.contains_key("model_armor_response"));
}

#[tokio::test]
async fn request_files_status_stays_blocked_and_the_response_appends() {
    let (endpoint, _requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    request_data.metadata = Some(MetadataMap::from([
        (STATUS_KEY.to_owned(), Json::Text("blocked".to_owned())),
        (
            RESPONSE_KEY.to_owned(),
            Json::Object(MetadataMap::from([(
                "sanitizationResult".to_owned(),
                Json::Object(MetadataMap::new()),
            )])),
        ),
    ]));
    let result = scan_request_files(
        &messages(&pdf_attachment()),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("blocked".to_owned()))
    );
    match metadata(&request_data).get(RESPONSE_KEY) {
        Some(Json::List(responses)) => assert_eq!(responses.len(), 2),
        other => panic!("expected two scan responses, got {other:?}"),
    }
}

#[tokio::test]
async fn request_files_continues_after_a_failed_attachment_when_fail_open() {
    let (endpoint, requests, _server) = serve(vec![
        (StatusCode::BAD_REQUEST, r#"{"error":"bad"}"#.to_owned()),
        (StatusCode::OK, BLOCKED_PII.to_owned()),
    ])
    .await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages_pair(
            &pdf_attachment(),
            &pdf_attachment().replace("aGVsbG8=", "d29ybGQ="),
        ),
        &mut request_data,
        &transport,
        &settings(Some("armor"), false, false),
    )
    .await;
    blocked(result.expect_err("the second attachment blocks"));
    assert_eq!(*requests.lock().expect("requests"), 2);
    assert_eq!(
        metadata(&request_data).get(STATUS_KEY),
        Some(&Json::Text("blocked".to_owned()))
    );
}

#[tokio::test]
async fn request_files_a_block_wins_over_a_later_clean_attachment() {
    let (endpoint, requests, _server) = serve(vec![
        (StatusCode::OK, BLOCKED_PII.to_owned()),
        (StatusCode::OK, CLEAN.to_owned()),
    ])
    .await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let result = scan_request_files(
        &messages_pair(
            &pdf_attachment(),
            &pdf_attachment().replace("aGVsbG8=", "d29ybGQ="),
        ),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    blocked(result.expect_err("the first attachment blocks"));
    assert_eq!(*requests.lock().expect("requests"), 1);
}

#[tokio::test]
async fn request_files_exactly_four_mib_is_scanned() {
    let (endpoint, requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let payload = STANDARD.encode(vec![b'a'; MAX_FILE_SIZE_BYTES]);
    let block = format!(
        r#"{{"type":"file","file":{{"file_data":"data:application/pdf;base64,{payload}"}}}}"#
    );
    let mut request_data = slots();
    let result = scan_request_files(
        &messages(&block),
        &mut request_data,
        &transport,
        &settings(Some("armor"), true, false),
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(*requests.lock().expect("requests"), 1);
}

#[tokio::test]
async fn request_files_warns_when_unscannable_attachments_are_skipped() {
    let (endpoint, _requests, _server) = serve(Vec::new()).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let log = logged(async {
        let result = scan_request_files(
            &messages(r#"{"type":"document","source":{"media_type":"application/pdf"}}"#),
            &mut request_data,
            &transport,
            &settings(Some("armor"), true, true),
        )
        .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(log.contains("skip_unscannable_attachments"));
}

#[tokio::test]
async fn request_files_does_not_warn_about_skipping_an_empty_count() {
    let (endpoint, _requests, _server) = serve(vec![(StatusCode::OK, CLEAN.to_owned())]).await;
    let transport = transport(&endpoint, true);
    let mut request_data = slots();
    let log = logged(async {
        let result = scan_request_files(
            &messages(&pdf_attachment()),
            &mut request_data,
            &transport,
            &settings(Some("armor"), true, true),
        )
        .await;
        assert!(result.is_ok());
    })
    .await;
    assert!(!log.contains("skip_unscannable_attachments"));
}
