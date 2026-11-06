#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::IntoFuture as _;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use chrono::NaiveDate;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::checkpoint::{CheckpointError, CheckpointRequest, DataType};
use super::environment::Settings;
use super::error::OvalixError;
use super::exception_status::GuardrailRaisedException;
use super::session_identity::{Clock, RequestData};
use super::{
    CHECKPOINT_PATH, CallHook, CallbackRegistry, InputType, Inputs, OvalixGuardrail, Params,
    initialize_guardrail,
};

const TODAY: &str = "2026-03-04";
const EMAIL_ACTOR_HASH: &str = "b4c9a289";
const UNKNOWN_ACTOR_HASH: &str = "b23a6a84";

#[derive(Debug)]
struct FixedClock(NaiveDate);

impl Clock for FixedClock {
    fn today(&self) -> NaiveDate {
        self.0
    }
}

fn today() -> NaiveDate {
    NaiveDate::parse_from_str(TODAY, "%Y-%m-%d").expect("date")
}

#[derive(Clone)]
struct Received {
    request: CheckpointRequest,
    authorization: String,
    content_type: String,
}

type Captured = Arc<Mutex<Vec<Received>>>;

struct Reply {
    status: StatusCode,
    body: String,
    delay: Duration,
}

fn ok(body: String) -> Reply {
    Reply {
        status: StatusCode::OK,
        body,
        delay: Duration::ZERO,
    }
}

fn delayed(body: String, delay: Duration) -> Reply {
    Reply {
        status: StatusCode::OK,
        body,
        delay,
    }
}

fn failed(status: StatusCode) -> Reply {
    Reply {
        status,
        body: String::new(),
        delay: Duration::ZERO,
    }
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn serve(replies: Vec<Reply>) -> (String, Captured, JoinSet<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let requests: Captured = Arc::default();
    let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
    let state = (Arc::clone(&requests), queue);
    let router = Router::new().route(
        CHECKPOINT_PATH,
        post(move |headers: HeaderMap, body: Bytes| {
            let (captured, pending) = state.clone();
            async move {
                let request: CheckpointRequest =
                    serde_json::from_slice(&body).expect("checkpoint request");
                captured.lock().expect("lock").push(Received {
                    request,
                    authorization: header(&headers, "authorization"),
                    content_type: header(&headers, "content-type"),
                });
                let next = pending.lock().expect("lock").pop_front();
                let reply = next.unwrap_or(Reply {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: String::new(),
                    delay: Duration::ZERO,
                });
                sleep(reply.delay).await;
                (reply.status, reply.body)
            }
        }),
    );
    let mut server = JoinSet::new();
    server.spawn(axum::serve(listener, router).into_future());
    (base, requests, server)
}

fn guardrail(base: &str) -> OvalixGuardrail<FixedClock> {
    OvalixGuardrail {
        settings: Settings {
            tracker_api_base: base.to_owned(),
            tracker_api_key: SecretString::from("oracle-key"),
            application_id: "app-1".to_owned(),
            pre_checkpoint_id: Some("pre-1".to_owned()),
            post_checkpoint_id: Some("post-1".to_owned()),
        },
        guardrail_name: "ovalix-test".to_owned(),
        default_on: false,
        event_hooks: vec![CallHook::PreCall, CallHook::PostCall],
        mode: vec![CallHook::PreCall],
        timeout: Duration::from_secs(5),
        client: reqwest::Client::default(),
        clock: FixedClock(today()),
    }
}

fn inputs(texts: &[&str]) -> Inputs {
    Inputs {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

fn email_request_data() -> RequestData {
    RequestData {
        metadata: Some(BTreeMap::from([(
            "user_api_key_user_email".to_owned(),
            Some("user@example.com".to_owned()),
        )])),
        litellm_metadata: None,
    }
}

fn corrected(content: &str) -> String {
    format!(r#"{{"action_type":"none","modified_data":{{"content":"{content}"}}}}"#)
}

fn blocked(content: Option<&str>) -> String {
    content.map_or_else(
        || r#"{"action_type":"block"}"#.to_owned(),
        |corrected| {
            format!(r#"{{"action_type":"Block","modified_data":{{"content":"{corrected}"}}}}"#)
        },
    )
}

const ALLOWED: &str = r#"{"action_type":"none"}"#;

#[tokio::test]
async fn checkpoint_payload_matches_the_tracker_contract() {
    let (base, requests, _server) = serve(vec![ok(ALLOWED.to_owned())]).await;
    let guardrail = guardrail(&base);
    guardrail
        .apply_guardrail(
            inputs(&["hello there"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect("allowed");
    let captured = requests.lock().expect("lock").clone();
    let [received] = captured.as_slice() else {
        panic!("one checkpoint request");
    };
    assert_eq!(
        received.request,
        CheckpointRequest {
            application_id: "app-1".to_owned(),
            checkpoint_id: "pre-1".to_owned(),
            actor: EMAIL_ACTOR_HASH.to_owned(),
            session_id: format!("{EMAIL_ACTOR_HASH}_{TODAY}_app-1"),
            data_type: DataType::Text,
            data: super::checkpoint::CheckpointData {
                content: "hello there".to_owned(),
            },
        }
    );
    assert_eq!(received.authorization, "Bearer oracle-key");
    assert_eq!(received.content_type, "application/json");
}

#[tokio::test]
async fn texts_are_sent_to_the_tracker_in_reverse_order() {
    let (base, requests, _server) = serve(vec![
        ok(ALLOWED.to_owned()),
        ok(ALLOWED.to_owned()),
        ok(ALLOWED.to_owned()),
    ])
    .await;
    let guardrail = guardrail(&base);
    let output = guardrail
        .apply_guardrail(
            inputs(&["alpha", "beta", "gamma"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect("allowed");
    assert_eq!(output.texts, vec!["alpha", "beta", "gamma"]);
    let contents: Vec<String> = requests
        .lock()
        .expect("lock")
        .iter()
        .map(|received| received.request.data.content.clone())
        .collect();
    assert_eq!(contents, vec!["gamma", "beta", "alpha"]);
}

#[tokio::test]
async fn corrections_replace_texts_in_their_original_order() {
    let (base, _requests, _server) = serve(vec![
        ok(corrected("GAMMA")),
        ok(corrected("BETA")),
        ok(ALLOWED.to_owned()),
    ])
    .await;
    let guardrail = guardrail(&base);
    let output = guardrail
        .apply_guardrail(
            inputs(&["alpha", "beta", "gamma"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect("allowed");
    assert_eq!(output.texts, vec!["alpha", "BETA", "GAMMA"]);
}

#[tokio::test]
async fn a_blocked_earlier_text_is_replaced_with_the_correction() {
    let (base, _requests, _server) = serve(vec![
        ok(ALLOWED.to_owned()),
        ok(blocked(Some("blocked earlier"))),
    ])
    .await;
    let guardrail = guardrail(&base);
    let output = guardrail
        .apply_guardrail(
            inputs(&["old", "current"]),
            &email_request_data(),
            InputType::Response,
        )
        .await
        .expect("allowed");
    assert_eq!(output.texts, vec!["blocked earlier", "current"]);
}

#[tokio::test]
async fn a_blocked_earlier_text_without_modified_data_uses_the_fallback_message() {
    let (base, _requests, _server) = serve(vec![ok(ALLOWED.to_owned()), ok(blocked(None))]).await;
    let guardrail = guardrail(&base);
    let output = guardrail
        .apply_guardrail(
            inputs(&["old", "current"]),
            &email_request_data(),
            InputType::Response,
        )
        .await
        .expect("allowed");
    assert_eq!(
        output.texts,
        vec![super::BLOCKED_FALLBACK_MESSAGE, "current"]
    );
}

#[tokio::test]
async fn a_blocked_current_text_raises_the_blocked_exception() {
    let (base, _requests, _server) = serve(vec![ok(blocked(Some("no")))]).await;
    let guardrail = guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["old", "current"]),
            &email_request_data(),
            InputType::Response,
        )
        .await
        .expect_err("blocked");
    assert!(
        matches!(error, OvalixError::Blocked(exception) if exception.message == "no" && exception.guardrail_name == "ovalix-test")
    );
}

#[tokio::test]
async fn a_blocked_current_text_without_modified_data_uses_the_fallback_message() {
    let (base, _requests, _server) = serve(vec![ok(blocked(None))]).await;
    let guardrail = guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["current"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect_err("blocked");
    assert!(
        matches!(error, OvalixError::Blocked(exception) if exception.message == super::BLOCKED_FALLBACK_MESSAGE)
    );
}

#[tokio::test]
async fn a_non_object_modified_data_counts_as_absent() {
    let (base, _requests, _server) = serve(vec![
        ok(ALLOWED.to_owned()),
        ok(r#"{"action_type":"none","modified_data":"not a dict"}"#.to_owned()),
        ok(r#"{"action_type":"none","modified_data":["injected"]}"#.to_owned()),
    ])
    .await;
    let guardrail = guardrail(&base);
    let output = guardrail
        .apply_guardrail(
            inputs(&["oldest", "middle", "current"]),
            &email_request_data(),
            InputType::Response,
        )
        .await
        .expect("allowed");
    assert_eq!(output.texts, vec!["oldest", "middle", "current"]);
}

#[tokio::test]
async fn a_slow_tracker_becomes_a_timeout_error() {
    let (base, _requests, _server) = serve(vec![delayed(
        ALLOWED.to_owned(),
        Duration::from_millis(300),
    )])
    .await;
    let mut guardrail = guardrail(&base);
    guardrail.timeout = Duration::from_millis(50);
    let error = guardrail
        .apply_guardrail(
            inputs(&["prompt"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect_err("timeout");
    assert!(matches!(
        error,
        OvalixError::Raised(GuardrailRaisedException {
            source: CheckpointError::Timeout { .. },
            ..
        })
    ));
}

#[tokio::test]
async fn a_tracker_response_over_the_size_cap_is_rejected() {
    let oversized = "x".repeat(super::MAX_RESPONSE_BYTES + 1);
    let (base, _requests, _server) = serve(vec![ok(oversized)]).await;
    let guardrail = guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["prompt"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect_err("oversized");
    assert!(matches!(
        error,
        OvalixError::Raised(GuardrailRaisedException {
            source: CheckpointError::ResponseTooLarge { limit },
            ..
        }) if limit == super::MAX_RESPONSE_BYTES
    ));
}

#[tokio::test]
async fn a_response_is_left_alone_without_a_post_checkpoint_id() {
    let mut guardrail = guardrail("http://127.0.0.1:9");
    guardrail.settings.post_checkpoint_id = None;
    let output = guardrail
        .apply_guardrail(
            inputs(&["answer"]),
            &email_request_data(),
            InputType::Response,
        )
        .await
        .expect("untouched");
    assert_eq!(output.texts, vec!["answer"]);
}

#[tokio::test]
async fn a_request_is_left_alone_without_a_pre_checkpoint_id() {
    let mut guardrail = guardrail("http://127.0.0.1:9");
    guardrail.settings.pre_checkpoint_id = None;
    let output = guardrail
        .apply_guardrail(
            inputs(&["prompt"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect("untouched");
    assert_eq!(output.texts, vec!["prompt"]);
}

#[tokio::test]
async fn an_empty_text_list_returns_the_inputs_unchanged() {
    let (base, requests, _server) = serve(Vec::new()).await;
    let guardrail = guardrail(&base);
    let mut original = inputs(&[]);
    original.model = Some("model".to_owned());
    let output = guardrail
        .apply_guardrail(original.clone(), &email_request_data(), InputType::Request)
        .await
        .expect("untouched");
    assert_eq!(output, original);
    assert!(requests.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn other_input_fields_survive_a_checkpoint_round_trip() {
    let (base, _requests, _server) = serve(vec![ok(corrected("fixed"))]).await;
    let guardrail = guardrail(&base);
    let mut original = inputs(&["answer"]);
    original.model = Some("gpt-4o".to_owned());
    original.images = Some(vec!["https://example.test/image.png".to_owned()]);
    let output = guardrail
        .apply_guardrail(original, &email_request_data(), InputType::Response)
        .await
        .expect("allowed");
    assert_eq!(output.texts, vec!["fixed"]);
    assert_eq!(output.model.as_deref(), Some("gpt-4o"));
    assert_eq!(output.images.expect("images").len(), 1);
}

#[tokio::test]
async fn a_tracker_error_status_becomes_a_guardrail_raised_error() {
    let (base, _requests, _server) = serve(vec![failed(StatusCode::INTERNAL_SERVER_ERROR)]).await;
    let guardrail = guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["prompt"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect_err("server error");
    assert!(
        matches!(&error, OvalixError::Raised(exception) if exception.guardrail_name == "ovalix-test")
    );
    assert!(error.to_string().starts_with("Ovalix guardrail error: "));
}

#[tokio::test]
async fn a_tracker_body_that_is_not_json_becomes_a_guardrail_raised_error() {
    let (base, _requests, _server) = serve(vec![ok("not json".to_owned())]).await;
    let guardrail = guardrail(&base);
    let error = guardrail
        .apply_guardrail(
            inputs(&["prompt"]),
            &email_request_data(),
            InputType::Request,
        )
        .await
        .expect_err("bad body");
    assert!(matches!(error, OvalixError::Raised(_)));
}

#[test]
fn prepare_headers_uses_the_tracker_bearer_token() {
    let guardrail = guardrail("http://127.0.0.1:9");
    let headers = guardrail.prepare_headers().expect("headers");
    assert_eq!(headers["content-type"], "application/json");
    let authorization = headers["authorization"].to_str().expect("header text");
    assert_eq!(authorization, "Bearer oracle-key");
    assert!(headers["authorization"].is_sensitive());
    assert!(!format!("{:?}", guardrail.settings.tracker_api_key).contains("oracle-key"));
}

#[test]
fn tracker_actor_id_hashes_the_unknown_actor() {
    let data = RequestData::default();
    assert_eq!(
        super::session_identity::tracker_actor_id(&data),
        UNKNOWN_ACTOR_HASH
    );
    assert_eq!(super::session_identity::actor(&data), "unknown");
}

#[derive(Default)]
struct Registry {
    registered: Option<Arc<OvalixGuardrail<FixedClock>>>,
}

impl CallbackRegistry<FixedClock> for Registry {
    fn add_litellm_callback(&mut self, guardrail: Arc<OvalixGuardrail<FixedClock>>) {
        self.registered = Some(guardrail);
    }
}

fn environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "OVALIX_TRACKER_API_BASE".to_owned(),
            "http://tracker.test".to_owned(),
        ),
        ("OVALIX_TRACKER_API_KEY".to_owned(), "oracle-key".to_owned()),
        ("OVALIX_APPLICATION_ID".to_owned(), "app-1".to_owned()),
        ("OVALIX_PRE_CHECKPOINT_ID".to_owned(), "pre-1".to_owned()),
        ("OVALIX_POST_CHECKPOINT_ID".to_owned(), "post-1".to_owned()),
    ])
}

fn params() -> Params {
    Params {
        guardrail_name: Some("ovalix".to_owned()),
        tracker_api_base: None,
        tracker_api_key: None,
        application_id: None,
        pre_checkpoint_id: None,
        post_checkpoint_id: None,
        event_hooks: Vec::new(),
        mode: Vec::new(),
        default_on: true,
        timeout: None,
    }
}

#[test]
fn the_configured_mode_stays_beside_the_supported_hooks() {
    let mut configured = params();
    configured.mode = vec![CallHook::PreCall];
    let mut registry = Registry::default();
    let guardrail = initialize_guardrail(
        configured,
        &environment(),
        FixedClock(today()),
        reqwest::Client::default(),
        &mut registry,
    )
    .expect("initialized");
    assert_eq!(guardrail.mode, vec![CallHook::PreCall]);
    assert_eq!(
        guardrail.event_hooks,
        vec![CallHook::PreCall, CallHook::PostCall]
    );
}

#[test]
fn initialize_guardrail_resolves_the_environment_and_registers_the_callback() {
    let mut registry = Registry::default();
    let guardrail = initialize_guardrail(
        params(),
        &environment(),
        FixedClock(today()),
        reqwest::Client::default(),
        &mut registry,
    )
    .expect("initialized");
    assert_eq!(guardrail.settings.tracker_api_base, "http://tracker.test");
    assert_eq!(
        guardrail.settings.tracker_api_key.expose_secret(),
        "oracle-key"
    );
    assert_eq!(guardrail.settings.application_id, "app-1");
    assert_eq!(guardrail.guardrail_name, "ovalix");
    assert!(guardrail.default_on);
    assert_eq!(guardrail.event_hooks, super::SUPPORTED_EVENT_HOOKS);
    assert_eq!(guardrail.timeout, Duration::from_secs(600));
    let registered = registry.registered.expect("registered");
    assert!(Arc::ptr_eq(&registered, &guardrail));
}

#[test]
fn initialize_guardrail_prefers_explicit_params_over_the_environment() {
    let mut explicit = params();
    explicit.tracker_api_base = Some("http://explicit.test".to_owned());
    explicit.tracker_api_key = Some(SecretString::from("explicit-key"));
    explicit.application_id = Some("explicit-app".to_owned());
    explicit.pre_checkpoint_id = Some("explicit-pre".to_owned());
    explicit.post_checkpoint_id = Some("explicit-post".to_owned());
    explicit.timeout = Some(Duration::from_millis(250));
    let mut registry = Registry::default();
    let guardrail = initialize_guardrail(
        explicit,
        &environment(),
        FixedClock(today()),
        reqwest::Client::default(),
        &mut registry,
    )
    .expect("initialized");
    assert_eq!(guardrail.settings.tracker_api_base, "http://explicit.test");
    assert_eq!(
        guardrail.settings.tracker_api_key.expose_secret(),
        "explicit-key"
    );
    assert_eq!(
        guardrail.settings.pre_checkpoint_id.as_deref(),
        Some("explicit-pre")
    );
    assert_eq!(guardrail.timeout, Duration::from_millis(250));
}

#[test]
fn initialize_guardrail_reports_missing_settings() {
    let mut registry = Registry::default();
    let error = initialize_guardrail(
        params(),
        &BTreeMap::new(),
        FixedClock(today()),
        reqwest::Client::default(),
        &mut registry,
    )
    .expect_err("missing");
    assert!(
        error
            .missing
            .contains(&"tracker_api_base (OVALIX_TRACKER_API_BASE)")
    );
    assert!(
        error
            .missing
            .contains(&"application_id (OVALIX_APPLICATION_ID)")
    );
    assert!(registry.registered.is_none());
}
