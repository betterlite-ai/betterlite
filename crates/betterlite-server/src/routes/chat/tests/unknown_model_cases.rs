use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use axum::Extension;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_config::loader::ConfigLoader;
use betterlite_core::dispatch::error::DispatchError;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde::Deserialize;
use tower::ServiceExt as _;

use crate::route_read_through::ModelReadThrough;
use crate::routes::chat::{CallContext, ErrorLayer, chat_routes_with_call_id, dispatch_failure};
use crate::server::chat_setup::chat_executor;
use crate::state::ServerState;
use crate::state::shared::Shared;

const SOURCE: &str = "model_list:\n  - {model_name: g, litellm_params: {model: openai/up, api_base: 'http://127.0.0.1:9/v1'}}\n";

struct RecordingReadThrough {
    answer: bool,
    asked: Mutex<Vec<String>>,
}

impl RecordingReadThrough {
    fn new(answer: bool) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModelReadThrough for RecordingReadThrough {
    async fn attempt_model(&self, model: &str) -> bool {
        self.asked.lock().unwrap().push(model.to_owned());
        self.answer
    }
}

#[derive(Deserialize)]
struct NotFoundReply {
    error: NotFoundFields,
}

#[derive(Deserialize)]
struct NotFoundFields {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: String,
    provider_specific_fields: Option<RouterFields>,
}

#[derive(Deserialize)]
struct RouterFields {
    error: String,
}

fn litellm_text(model: &str) -> String {
    format!(
        "/chat/completions: Invalid model name passed in model={model}. Call `/v1/models` to view available models for your key."
    )
}

async fn ask(
    path: &str,
    body: &str,
    read_through: Option<Arc<dyn ModelReadThrough>>,
) -> (u16, NotFoundReply) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(SOURCE)
    .expect("config");
    let environment = BTreeMap::from([("OPENAI_API_KEY".to_owned(), "env-key".to_owned())]);
    let executor =
        chat_executor(&config, &environment, StdRng::seed_from_u64(3)).expect("executor");
    let shared = Arc::new(Shared {
        bundled_catalog: Arc::clone(&executor.bundled_catalog),
    });
    let mut state = ServerState::new(&config, Arc::new(executor), shared);
    state.model_read_through.read_through = read_through;
    let shared_state = Arc::new(state);
    let router = chat_routes_with_call_id(Arc::clone(&shared_state.executor), false)
        .expect("routes")
        .layer(Extension(Arc::clone(&shared_state)));
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request");
    let response = router.oneshot(request).await.expect("response");
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (status, serde_json::from_slice(&bytes).expect("error body"))
}

#[tokio::test]
async fn unknown_model_answers_litellm_text_on_the_v1_path() {
    let (status, reply) = ask(
        "/v1/chat/completions",
        r#"{"model":"nope","messages":[]}"#,
        None,
    )
    .await;
    assert_eq!(status, 400);
    let text = litellm_text("nope");
    assert_eq!(reply.error.message, text);
    assert_eq!(reply.error.error_type, "invalid_request_error");
    assert_eq!(reply.error.param, None);
    assert_eq!(reply.error.code, "400");
    assert_eq!(
        reply
            .error
            .provider_specific_fields
            .map(|fields| fields.error),
        Some(text)
    );
}

#[tokio::test]
async fn unknown_model_answers_litellm_text_on_an_engines_path() {
    let (status, reply) = ask(
        "/engines/nope/chat/completions",
        r#"{"model":"body-model","messages":[]}"#,
        None,
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply.error.message, litellm_text("nope"));
}

#[tokio::test]
async fn unknown_model_retries_once_when_the_read_through_answers() {
    let recorder = RecordingReadThrough::new(true);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask(
        "/v1/chat/completions",
        r#"{"model":"nope","messages":[]}"#,
        Some(port),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply.error.message, litellm_text("nope"));
    assert_eq!(recorder.asked(), vec![String::from("nope")]);
}

#[tokio::test]
async fn unknown_model_keeps_the_first_answer_when_the_read_through_declines() {
    let recorder = RecordingReadThrough::new(false);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask(
        "/v1/chat/completions",
        r#"{"model":"nope","messages":[]}"#,
        Some(port),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(reply.error.message, litellm_text("nope"));
    assert_eq!(recorder.asked(), vec![String::from("nope")]);
}

#[tokio::test]
async fn unknown_model_without_a_model_never_asks_the_read_through() {
    let recorder = RecordingReadThrough::new(true);
    let port: Arc<dyn ModelReadThrough> = Arc::clone(&recorder);
    let (status, reply) = ask("/v1/chat/completions", r#"{"messages":[]}"#, Some(port)).await;
    assert_eq!(status, 400);
    assert_eq!(reply.error.message, litellm_text("None"));
    assert!(recorder.asked().is_empty());
}

#[tokio::test]
async fn unknown_model_redacts_a_key_like_model() {
    let (status, reply) = ask(
        "/v1/chat/completions",
        r#"{"model":"Bearer sk-secretsecretsecretsecret","messages":[]}"#,
        None,
    )
    .await;
    assert_eq!(status, 400);
    let repeated = reply
        .error
        .provider_specific_fields
        .map(|fields| fields.error);
    assert_eq!(repeated.as_deref(), Some(reply.error.message.as_str()));
    assert!(!reply.error.message.contains("sk-secretsecretsecretsecret"));
}

#[tokio::test]
async fn an_undispatchable_chat_call_answers_internal_server_error() {
    let errors = ErrorLayer::new().expect("errors");
    let call = CallContext {
        id: "call-internal",
        include_in_body: false,
        started: Instant::now(),
        errors: &errors,
    };
    let response = dispatch_failure(DispatchError::NotBuilt { entry: "chat" }, &call, None);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let reply: NotFoundReply = serde_json::from_slice(&bytes).expect("error body");
    assert_eq!(reply.error.message, "the chat call could not be dispatched");
    assert_eq!(reply.error.error_type, "internal_server_error");
    assert_eq!(reply.error.code, "500");
    assert_eq!(reply.error.param, None);
    assert_eq!(
        reply
            .error
            .provider_specific_fields
            .map(|fields| fields.error),
        None
    );
}
