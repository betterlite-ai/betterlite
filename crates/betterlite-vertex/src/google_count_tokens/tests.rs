#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use std::{fs, io};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::studio_identity::{DEFAULT_API_BASE, Settings};
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use http::Method;
use http::header::HeaderMap;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::credential_sources::getters::VertexParams;
use crate::credential_sources::sdk::CredentialClock;
use crate::google_count_tokens::PROVIDER;
use crate::google_count_tokens::cleaner::clean_contents;
use crate::google_count_tokens::decode::{MAX_COUNT_BYTES, decode_google_count};
use crate::google_count_tokens::error::GoogleCountError;
use crate::google_count_tokens::studio::{StudioCountInput, count_tokens};
use crate::google_count_tokens::transport::CountTransport;
use crate::google_count_tokens::vertex::{VertexCountInput, count_tokens as count_vertex_tokens};
use crate::studio_identity::NativeParams;
use crate::token_cache::CredentialCache;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::executor::{CredentialExecutor, LoadedCredential};
use crate::token_cache::record::CredentialSnapshot;
use crate::token_cache::state::TokenState;
use crate::token_refresh::CredentialRefresh;

const DEADLINE: Duration = Duration::from_secs(5);

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-count-probe:countTokens";

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[test]
fn projection_removes_the_function_response_id() {
    let contents = json(
        r#"[{"role":"user","parts":[{"text":"hi"}]},{"role":"user","parts":[{"functionResponse":{"id":"call-1","name":"read","response":{"output":"ok"}}}]}]"#,
    );
    let cleaned = clean_contents(&contents).unwrap();
    assert_eq!(
        cleaned,
        json(
            r#"[{"role":"user","parts":[{"text":"hi"}]},{"role":"user","parts":[{"functionResponse":{"name":"read","response":{"output":"ok"}}}]}]"#
        )
    );
}

#[test]
fn projection_does_not_mutate_the_input() {
    let contents = json(
        r#"[{"parts":[{"functionResponse":{"id":"call-1","name":"read","response":{"output":"ok"}}}]}]"#,
    );
    let before = contents.clone();
    let cleaned = clean_contents(&contents).unwrap();
    assert_eq!(contents, before);
    assert_ne!(cleaned, contents);
}

#[test]
fn projection_keeps_falsey_contents() {
    assert_eq!(clean_contents(&Json::Null).unwrap(), Json::Null);
    let empty = json("[]");
    assert_eq!(clean_contents(&empty).unwrap(), empty);
}

#[test]
fn projection_keeps_content_records_without_function_responses() {
    let contents = json(r#"[{"role":"model","parts":[{"text":"kept"},{"thought":true}]}]"#);
    assert_eq!(clean_contents(&contents).unwrap(), contents);
}

#[test]
fn projection_projects_both_field_spellings_and_drops_none() {
    let contents = json(
        r#"[{"parts":[{"functionResponse":{"willContinue":true,"scheduling":"when_idle","name":null,"response":{},"id":"c"}}]}]"#,
    );
    assert_eq!(
        clean_contents(&contents).unwrap(),
        json(
            r#"[{"parts":[{"functionResponse":{"will_continue":true,"scheduling":"WHEN_IDLE","response":{}}}]}]"#
        )
    );
    let snake_case = json(
        r#"[{"parts":[{"functionResponse":{"will_continue":false,"scheduling":"custom","name":"f"}}]}]"#,
    );
    assert_eq!(
        clean_contents(&snake_case).unwrap(),
        json(
            r#"[{"parts":[{"functionResponse":{"will_continue":false,"scheduling":"custom","name":"f"}}]}]"#
        )
    );
}

#[test]
fn projection_keeps_function_response_parts_as_the_caller_sent_them() {
    let contents = json(
        r#"[{"parts":[{"functionResponse":{"name":"f","parts":[{"inlineData":{"mimeType":"text/plain","data":"aGk="}},{"fileData":{"fileUri":"gs://b/x"}}]}}]}]"#,
    );
    assert_eq!(
        clean_contents(&contents).unwrap(),
        json(
            r#"[{"parts":[{"functionResponse":{"parts":[{"inlineData":{"mimeType":"text/plain","data":"aGk="}},{"fileData":{"fileUri":"gs://b/x"}}],"name":"f"}}]}]"#
        )
    );
}

#[test]
fn projection_rejects_an_undeclared_function_response_field() {
    let contents = json(r#"[{"parts":[{"functionResponse":{"name":"f","bogus":1}}]}]"#);
    match clean_contents(&contents) {
        Err(GoogleCountError::FunctionResponse { source, .. }) => {
            assert!(source.to_string().contains("bogus"));
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn projection_names_a_wrongly_typed_field() {
    let contents = json(r#"[{"parts":[{"functionResponse":{"name":7}}]}]"#);
    match clean_contents(&contents) {
        Err(GoogleCountError::FunctionResponse { path, source }) => {
            assert!(path.contains("name"));
            assert!(source.to_string().contains("invalid type"));
        }
        other => panic!("unexpected result: {other:?}"),
    }
    let response = json(r#"[{"parts":[{"functionResponse":{"response":[1]}}]}]"#);
    match clean_contents(&response) {
        Err(GoogleCountError::FunctionResponse { path, .. }) => {
            assert!(path.contains("response"));
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn projection_rejects_shapes_litellm_cannot_clean() {
    assert!(matches!(
        clean_contents(&json(r#"{"parts":[]}"#)),
        Err(GoogleCountError::Contents { .. })
    ));
    assert!(matches!(
        clean_contents(&json(r#"[{"role":"user"}]"#)),
        Err(GoogleCountError::Contents { .. })
    ));
    assert!(matches!(
        clean_contents(&json(r#"[{"parts":"x"}]"#)),
        Err(GoogleCountError::Contents { .. })
    ));
    assert!(matches!(
        clean_contents(&json(r#"["x"]"#)),
        Err(GoogleCountError::Contents { .. })
    ));
    assert!(matches!(
        clean_contents(&json(r#"[{"parts":["x"]}]"#)),
        Err(GoogleCountError::Contents { .. })
    ));
    assert!(matches!(
        clean_contents(&json(r#"[{"parts":[{"functionResponse":null}]}]"#)),
        Err(GoogleCountError::FunctionResponse { .. })
    ));
}

#[test]
fn count_projection_never_panics_on_arbitrary_json() {
    for text in [
        "null",
        "0",
        "false",
        r#""""#,
        "[]",
        "[{}]",
        r#"[{"parts":[]}]"#,
        r#"[{"parts":[{}]}]"#,
        r#"[{"parts":[{"functionResponse":{}}]}]"#,
        r#"[{"parts":[{"functionResponse":{"parts":[]}}]}]"#,
        r#"[{"parts":[{"functionResponse":{"scheduling":"SILENT"}}]}]"#,
    ] {
        let _result = clean_contents(&json(text));
    }
}

#[derive(Clone)]
enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Failure,
}

#[derive(Clone)]
struct Captured {
    method: Method,
    url: String,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

struct Script {
    reply: Reply,
    requests: Mutex<Vec<Captured>>,
}

impl Script {
    fn replying(status: u16, body: &[u8]) -> Self {
        Self {
            reply: Reply::Answer {
                status,
                body: body.to_vec(),
            },
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            reply: Reply::Failure,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Captured> {
        self.requests.lock().unwrap().clone()
    }
}

impl CountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let body = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty
            | RequestBody::Bytes {
                content_type: _,
                data: _,
            }
            | RequestBody::Multipart {
                boundary: _,
                parts: _,
            } => Vec::new(),
        };
        self.requests.lock().unwrap().push(Captured {
            method: request.method,
            url: request.url.to_string(),
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        let reply = self.reply.clone();
        ready(match reply {
            Reply::Answer { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

fn studio_settings() -> Settings {
    Settings {
        environment_key: Some(SecretString::from("env-key")),
        generic_api_key: None,
        api_base: DEFAULT_API_BASE.to_owned(),
    }
}

#[tokio::test]
async fn http_count_post_body_sends_only_the_cleaned_contents() {
    let script = Script::replying(
        200,
        br#"{"totalTokens":7,"totalBillableCharacters":3,"promptTokensDetails":[{"modality":"TEXT","tokenCount":7}],"tokenizer_used":"google"}"#,
    );
    let contents =
        json(r#"[{"parts":[{"functionResponse":{"id":"c","name":"f","response":{"a":1}}}]}]"#);
    let settings = studio_settings();
    let params = NativeParams {
        api_key: None,
        gemini_api_key: None,
    };
    let input = StudioCountInput {
        model: "gemini-2.0-flash",
        contents: &contents,
        api_key: None,
        api_base: Some("https://proxy.example/"),
        params: &params,
        settings: &settings,
        timeout: Some(Duration::from_millis(1)),
    };
    let reply = count_tokens(&script, &input, DEADLINE).await.unwrap();
    assert_eq!(reply.total_tokens, Some(7));
    assert_eq!(reply.total_billable_characters, Some(3));
    assert_eq!(reply.tokenizer_used.as_deref(), Some("google"));
    let details = reply.prompt_tokens_details.unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].modality, "TEXT");
    assert_eq!(details[0].token_count, 7);
    let sent = script.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(
        sent[0].url,
        "https://proxy.example//v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(sent[0].deadline, DEADLINE);
    assert_eq!(
        sent[0].headers["x-goog-api-key"].to_str().unwrap(),
        "env-key"
    );
    assert!(sent[0].headers["x-goog-api-key"].is_sensitive());
    assert_eq!(
        sent[0].headers["Content-Type"].to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        serde_json::from_slice::<Json>(&sent[0].body).unwrap(),
        json(r#"{"contents":[{"parts":[{"functionResponse":{"name":"f","response":{"a":1}}}]}]}"#)
    );
}

#[tokio::test]
async fn http_count_defaults_the_studio_base_and_requests_its_key() {
    let script = Script::replying(200, br#"{"totalTokens":1}"#);
    let contents = json(r#"[{"parts":[{"text":"hi"}]}]"#);
    let settings = studio_settings();
    let params = NativeParams {
        api_key: None,
        gemini_api_key: None,
    };
    let key = SecretString::from("request-key");
    let input = StudioCountInput {
        model: "gemini-2.0-flash",
        contents: &contents,
        api_key: Some(&key),
        api_base: None,
        params: &params,
        settings: &settings,
        timeout: None,
    };
    count_tokens(&script, &input, DEADLINE).await.unwrap();
    let sent = script.requests();
    assert_eq!(
        sent[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:countTokens"
    );
    assert_eq!(
        sent[0].headers["x-goog-api-key"].to_str().unwrap(),
        "request-key"
    );
}

#[tokio::test]
async fn http_count_error_kind_keeps_the_status_and_the_connection_split() {
    let contents = json("[]");
    let settings = studio_settings();
    let params = NativeParams {
        api_key: None,
        gemini_api_key: None,
    };
    let input = StudioCountInput {
        model: "gemini-2.0-flash",
        contents: &contents,
        api_key: None,
        api_base: None,
        params: &params,
        settings: &settings,
        timeout: None,
    };
    let failed = Script::replying(500, b"boom");
    match count_tokens(&failed, &input, DEADLINE).await {
        Err(GoogleCountError::Status { status, body }) => {
            assert_eq!(status, 500);
            assert_eq!(body, "boom");
        }
        other => panic!("unexpected result: {other:?}"),
    }
    let rejected = Script::replying(400, b"bad request");
    match count_tokens(&rejected, &input, DEADLINE).await {
        Err(error @ GoogleCountError::Status { status: 400, .. }) => {
            assert_eq!(error.status_code(), 400);
        }
        other => panic!("unexpected result: {other:?}"),
    }
    let closed = Script::failing();
    match count_tokens(&closed, &input, DEADLINE).await {
        Err(error @ GoogleCountError::Connection { .. }) => {
            assert_eq!(error.status_code(), 500);
        }
        other => panic!("unexpected result: {other:?}"),
    }
    let created = Script::replying(201, br#"{"totalTokens":2}"#);
    assert_eq!(
        count_tokens(&created, &input, DEADLINE)
            .await
            .unwrap()
            .total_tokens,
        Some(2)
    );
    assert_eq!(PROVIDER, "gemini");
}

#[tokio::test]
async fn http_count_uses_the_real_client_transport() {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("an empty proxy policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("an outbound client");
    let contents = json("[]");
    let settings = studio_settings();
    let params = NativeParams {
        api_key: None,
        gemini_api_key: None,
    };
    let input = StudioCountInput {
        model: "gemini-2.0-flash",
        contents: &contents,
        api_key: None,
        api_base: Some("http://127.0.0.1:1"),
        params: &params,
        settings: &settings,
        timeout: None,
    };
    let error = count_tokens(&client, &input, DEADLINE)
        .await
        .expect_err("nothing listens on the chosen port");
    assert!(matches!(error, GoogleCountError::Connection { .. }));
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct StaticExecutor;

impl CredentialExecutor for StaticExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        let snapshot = CredentialSnapshot {
            token: Some(SecretString::from("vertex-token")),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some("proj".to_owned()),
        };
        Box::pin(ready(Ok(LoadedCredential {
            credential: Some(Arc::new(snapshot)),
            project: Some("proj".to_owned()),
        })))
    }

    fn refresh<'borrow>(
        &'borrow self,
        _credential: &'borrow CredentialSnapshot,
    ) -> Pin<
        Box<dyn Future<Output = Result<CredentialSnapshot, CredentialCacheError>> + Send + 'borrow>,
    > {
        Box::pin(ready(Err(CredentialCacheError::MissingToken)))
    }
}

fn refresh() -> CredentialRefresh {
    CredentialRefresh::new(
        Arc::new(CredentialCache::new(
            Arc::new(FixedClock),
            Arc::new(StaticExecutor),
        )),
        TaskTracker::new(),
    )
}

#[tokio::test]
async fn http_vertex_ignores_base_and_caller_headers() {
    let script = Script::replying(200, br#"{"totalTokens":4}"#);
    let contents = json("[]");
    let catalog = BundledCatalog::default().get();
    let settings = VertexSettings::default();
    let params = VertexParams {
        project: Some("proj"),
        ai_project: None,
        location: Some("us-central1"),
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    let ignored_headers = BTreeMap::from([
        ("X-Caller".to_owned(), "kept".to_owned()),
        ("Authorization".to_owned(), "Bearer caller".to_owned()),
    ]);
    let key = SecretString::from("caller-key");
    let input = VertexCountInput {
        model: "gemini-count-probe",
        contents: &contents,
        api_key: Some(&key),
        api_base: Some("https://caller.example/v1"),
        headers: &ignored_headers,
        params,
        settings: &settings,
        timeout: Some(Duration::from_millis(1)),
    };
    let refresh = refresh();
    count_vertex_tokens(&script, &refresh, &catalog, &input, DEADLINE)
        .await
        .unwrap();
    let sent = script.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(sent[0].url, VERTEX_URL);
    assert_eq!(sent[0].deadline, DEADLINE);
    assert!(sent[0].headers.get("x-goog-api-key").is_none());
    assert!(sent[0].headers.get("X-Caller").is_none());
}

#[tokio::test]
async fn http_vertex_emits_only_bearer_auth() {
    let script = Script::replying(200, br#"{"totalTokens":4}"#);
    let contents = json("[]");
    let catalog = BundledCatalog::default().get();
    let settings = VertexSettings::default();
    let params = VertexParams {
        project: Some("proj"),
        ai_project: None,
        location: Some("us-central1"),
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    let ignored_headers = BTreeMap::new();
    let key = SecretString::from("caller-key");
    let input = VertexCountInput {
        model: "gemini-count-probe",
        contents: &contents,
        api_key: Some(&key),
        api_base: None,
        headers: &ignored_headers,
        params,
        settings: &settings,
        timeout: None,
    };
    let refresh = refresh();
    count_vertex_tokens(&script, &refresh, &catalog, &input, DEADLINE)
        .await
        .unwrap();
    let sent = script.requests();
    assert_eq!(sent[0].headers.len(), 1);
    let authorization = &sent[0].headers["Authorization"];
    assert_eq!(authorization.to_str().unwrap(), "Bearer vertex-token");
    assert!(authorization.is_sensitive());
    assert_eq!(PROVIDER, "gemini");
}

#[tokio::test]
async fn http_vertex_status_error_keeps_the_inherited_gemini_provider() {
    let script = Script::replying(404, b"missing");
    let contents = json("[]");
    let catalog = BundledCatalog::default().get();
    let settings = VertexSettings::default();
    let params = VertexParams {
        project: Some("proj"),
        ai_project: None,
        location: Some("us-central1"),
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };
    let ignored_headers = BTreeMap::new();
    let input = VertexCountInput {
        model: "gemini-count-probe",
        contents: &contents,
        api_key: None,
        api_base: None,
        headers: &ignored_headers,
        params,
        settings: &settings,
        timeout: None,
    };
    let refresh = refresh();
    match count_vertex_tokens(&script, &refresh, &catalog, &input, DEADLINE).await {
        Err(error @ GoogleCountError::Status { status: 404, .. }) => {
            assert_eq!(error.status_code(), 404);
            assert_eq!(PROVIDER, "gemini");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn decode_retains_the_counted_fields() {
    let decoded = decode_google_count(
        br#"{"totalTokens":31,"totalBillableCharacters":96,"promptTokensDetails":[{"modality":"TEXT","tokenCount":31}],"tokenizer_used":"google"}"#,
    )
    .unwrap();
    assert_eq!(decoded.total_tokens, Some(31));
    assert_eq!(decoded.total_billable_characters, Some(96));
    let details = decoded.prompt_tokens_details.unwrap();
    assert_eq!(details[0].modality, "TEXT");
    assert_eq!(details[0].token_count, 31);
    assert_eq!(decoded.tokenizer_used.as_deref(), Some("google"));
    let minimal = decode_google_count(br#"{"totalTokens":0}"#).unwrap();
    assert_eq!(minimal.total_tokens, Some(0));
    assert_eq!(minimal.total_billable_characters, None);
    assert!(minimal.prompt_tokens_details.is_none());
    assert!(minimal.tokenizer_used.is_none());
}

#[test]
fn decode_names_the_field_it_rejects() {
    match decode_google_count(br#"{"totalTokens":"many"}"#) {
        Err(GoogleCountError::Json { path, .. }) => assert_eq!(path, "totalTokens"),
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(matches!(
        decode_google_count(b"[]"),
        Err(GoogleCountError::Json { .. })
    ));
}

#[test]
fn decode_boundary_is_one_mibibyte() {
    assert_eq!(MAX_COUNT_BYTES, 1024 * 1024);
    let mut at_limit = br#"{"totalTokens":1}"#.to_vec();
    at_limit.resize(MAX_COUNT_BYTES, b' ');
    decode_google_count(&at_limit).expect("the reply fits the limit");
    let mut over_limit = br#"{"totalTokens":1}"#.to_vec();
    over_limit.resize(MAX_COUNT_BYTES + 1, b' ');
    assert!(matches!(
        decode_google_count(&over_limit),
        Err(GoogleCountError::ResponseTooLarge {
            limit: MAX_COUNT_BYTES
        })
    ));
}

#[test]
fn replays_the_google_count_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/google_count");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("utf-8 file name");
            let decoded = decode_google_count(&bytes);
            if name.starts_with("valid") {
                decoded.expect("this seed is a count reply");
            } else {
                decoded.expect_err("this seed is not a count reply");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
