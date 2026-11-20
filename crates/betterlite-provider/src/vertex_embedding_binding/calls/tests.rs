#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::pin::Pin;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_context::call_context::{CallContext, CallInputs};
use betterlite_core::dispatch::log::TracingCallLog;
use betterlite_core::dispatch::{Dispatch, NoRules};
use betterlite_core::operation::embedding::{
    EmbeddingCall, EmbeddingCallInput, EmbeddingFailure, EmbeddingRequest as DispatchRequest,
    EmbeddingResult,
};
use betterlite_core::operation::{CallResult, Operation};
use betterlite_core::vertex_embedding_dispatch::pin;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::chat::deployment::{ChatDeployment, Provider};
use betterlite_openai::embedding::{EmbeddingError, EmbeddingRequest};
use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::credential_sources::sdk::CredentialClock;
use betterlite_vertex::gemini_embedding_call::transport::EmbeddingTransport;
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use betterlite_vertex::token_cache::CredentialCache;
use betterlite_vertex::token_cache::error::CredentialCacheError;
use betterlite_vertex::token_cache::executor::{CredentialExecutor, LoadedCredential};
use betterlite_vertex::token_cache::record::CredentialSnapshot;
use betterlite_vertex::token_cache::state::TokenState;
use betterlite_vertex::token_refresh::CredentialRefresh;
use betterlite_vertex::vertex_embedding_call::post::{
    EmbeddingPost, PostError, PostFuture, PostReply, PostRequest,
};
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use url::Url;
use uuid::Uuid;

use super::VertexEmbeddingCalls;

const TOKEN: &str = "ya29.vertex-test-token";
const MODEL_ID: &str = "deployment-1";
const PINNED: &str = r#"{"vertex_project": "pinned-project", "vertex_location": "europe-west4", "api_base": "https://vertex.example"}"#;
const TEXT_ANSWER: &str = r#"{"predictions":[{"embeddings":{"values":[0.1,0.2],"statistics":{"token_count":2,"truncated":false}}}]}"#;
const MULTIMODAL_ANSWER: &str = r#"{"predictions":[{"textEmbedding":[0.5,0.25]}]}"#;
const EMBED_CONTENT_ANSWER: &str =
    r#"{"embedding":{"values":[0.5]},"usageMetadata":{"promptTokenCount":3,"totalTokenCount":4}}"#;
const TEXT_REQUEST: &str = r#"{"model": "embed", "input": "hello"}"#;

#[derive(Clone, Debug)]
struct Sent {
    url: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

// answers the scripted replies in order for both the prediction posts and the embedContent send
struct Wire {
    sent: Mutex<Vec<Sent>>,
    replies: Mutex<VecDeque<(u16, Vec<u8>)>>,
}

impl Wire {
    fn answering(status: u16, body: &str) -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::new(Vec::new()),
            replies: Mutex::new(VecDeque::from([(status, body.as_bytes().to_vec())])),
        })
    }

    fn record(&self, sent: Sent) -> (u16, Vec<u8>) {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the test scripted a reply")
    }

    fn only_request(&self) -> Sent {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(sent.len(), 1, "the call posts exactly once");
        sent[0].clone()
    }
}

impl EmbeddingPost for Wire {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.expose_secret().to_owned()))
            .collect();
        let (status, body) = self.record(Sent {
            url: request.url.to_owned(),
            headers,
            body: request.body.to_vec(),
        });
        Box::pin(ready(Ok::<PostReply, PostError>(PostReply {
            status,
            body,
        })))
    }
}

impl EmbeddingTransport for Wire {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().expect("a text header").to_owned(),
                )
            })
            .collect();
        let payload = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        let (status, body) = self.record(Sent {
            url: request.url.to_string(),
            headers,
            body: payload,
        });
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body,
        }))
    }
}

struct FixedClock;

impl CredentialClock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct FixedExecutor;

impl CredentialExecutor for FixedExecutor {
    fn load<'borrow>(
        &'borrow self,
        _credentials: Option<&'borrow ConfiguredCredentials>,
        _project: Option<&'borrow str>,
    ) -> Pin<
        Box<dyn Future<Output = Result<LoadedCredential, CredentialCacheError>> + Send + 'borrow>,
    > {
        let credential = Arc::new(CredentialSnapshot {
            token: Some(SecretString::from(TOKEN.to_owned())),
            state: Some(TokenState::Fresh),
            expires_at: None,
            expired: None,
            valid: None,
            quota_project_id: None,
            project_id: Some("executor-project".to_owned()),
        });
        Box::pin(ready(Ok(LoadedCredential {
            credential: Some(credential),
            project: Some("executor-project".to_owned()),
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

struct CharacterCount;

impl TextTokenCounter for CharacterCount {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}

fn params(source: &str) -> Extra {
    serde_json::from_str(source).expect("litellm params")
}

fn deployment(litellm_model: &str) -> ChatDeployment {
    ChatDeployment {
        model_group: "embed".to_owned(),
        litellm_model: litellm_model.to_owned(),
        provider: Provider::Unsupported {
            model: litellm_model.to_owned(),
        },
        api_base: "https://api.openai.com/v1".to_owned(),
        endpoint: Url::parse("https://api.openai.com/v1/embeddings").expect("a URL"),
        authorization: None,
        api_key: None,
        organization: None,
        timeout: Duration::from_secs(30),
        timeout_seconds: 30.0,
        model_id: MODEL_ID.to_owned(),
        weight: 1,
        rpm: 0,
        tpm: 0,
        global_drop_params: false,
    }
}

fn calls(wire: &Arc<Wire>, pinned: &str) -> VertexEmbeddingCalls<Wire> {
    let executor: Arc<dyn CredentialExecutor> = Arc::new(FixedExecutor);
    let cache = Arc::new(CredentialCache::new(Arc::new(FixedClock), executor));
    let refresh = Arc::new(CredentialRefresh::new(
        Arc::clone(&cache),
        TaskTracker::new(),
    ));
    VertexEmbeddingCalls {
        catalog: BundledCatalog::default().get(),
        cache,
        refresh,
        counter: Arc::new(CharacterCount),
        transport: Arc::clone(wire),
        vertex: VertexSettings::default(),
        embedding: EmbeddingSettings::default(),
        pinned: BTreeMap::from([(MODEL_ID.to_owned(), pin(&params(pinned)).expect("pins"))]),
    }
}

async fn embed(
    calls: &VertexEmbeddingCalls<Wire>,
    litellm_model: &str,
    request: &str,
) -> Result<EmbeddingResult, EmbeddingFailure> {
    let parsed = EmbeddingRequest::parse(request.as_bytes()).expect("an embeddings request");
    calls
        .embed(EmbeddingCallInput {
            deployment: deployment(litellm_model),
            request: &parsed,
        })
        .await
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON document")
}

fn body_text(sent: &Sent) -> &str {
    str::from_utf8(&sent.body).expect("a text body")
}

#[tokio::test]
async fn a_vertex_deployment_runs_through_the_dispatch_pipeline() {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let embedding: Arc<dyn EmbeddingCall> = Arc::new(calls(&wire, PINNED));
    let dispatch = Dispatch::new(
        vec![deployment("vertex_ai/text-embedding-005")],
        Arc::new(NoRules),
        Arc::new(TracingCallLog),
    );
    let context = CallContext::start(CallInputs {
        now: SystemTime::now,
        id: Uuid::nil,
        deadline: None,
        privacy: false,
    });
    let request = EmbeddingRequest::parse(TEXT_REQUEST.as_bytes()).expect("a request");
    let operation = Operation::Embedding(DispatchRequest {
        call_id: "call-1".to_owned(),
        model_id: MODEL_ID.to_owned(),
        request: Box::new(request),
        calls: embedding,
    });
    let Ok(CallResult::Embedding(result)) = dispatch.execute(operation, &context).await else {
        panic!("the dispatch pipeline answers the embedding");
    };
    assert_eq!(result.response.model, "text-embedding-005");
    assert_eq!(result.response.data.len(), 1);
    let sent = wire.only_request();
    assert_eq!(
        sent.url,
        "https://vertex.example/v1/projects/pinned-project/locations/europe-west4/publishers/google/models/text-embedding-005:predict"
    );
    assert_eq!(
        sent.headers.get("authorization").map(String::as_str),
        Some("Bearer ya29.vertex-test-token")
    );
}

#[test]
fn the_installed_call_names_no_deployment_labels_or_caller_headers() {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let caller = calls(&wire, PINNED);
    let parsed = EmbeddingRequest::parse(TEXT_REQUEST.as_bytes()).expect("an embeddings request");
    let input = caller
        .input(&EmbeddingCallInput {
            deployment: deployment("vertex_ai/text-embedding-005"),
            request: &parsed,
        })
        .expect("a call input");
    // the core seam carries neither, so LiteLLM's deployment labels and caller headers stop here
    assert!(input.litellm_params.is_none());
    assert!(input.extra_headers.is_empty());
    assert_eq!(input.timeout, Some(Duration::from_secs(30)));
    assert!(input.pinned.is_some());
}

#[rstest]
#[case::text(
    "vertex_ai/text-embedding-005",
    TEXT_REQUEST,
    TEXT_ANSWER,
    ":predict",
    r#"{"instances":[{"content":"hello"}],"parameters":{}}"#
)]
#[case::embed_content(
    "vertex_ai/gemini-embedding-2",
    TEXT_REQUEST,
    EMBED_CONTENT_ANSWER,
    ":embedContent",
    r#"{"content":{"parts":[{"text":"hello"}]}}"#
)]
#[case::multimodal_model(
    "vertex_ai/multimodalembedding@001",
    TEXT_REQUEST,
    MULTIMODAL_ANSWER,
    ":predict",
    r#"{"instances":[{"text":"hello"}]}"#
)]
#[tokio::test]
async fn each_arm_posts_its_own_request(
    #[case] litellm_model: &str,
    #[case] request: &str,
    #[case] answer: &str,
    #[case] verb: &str,
    #[case] expected_body: &str,
) {
    let wire = Wire::answering(200, answer);
    let result = embed(&calls(&wire, PINNED), litellm_model, request)
        .await
        .expect("an embedding");
    let upstream = litellm_model.split_once('/').expect("a provider prefix").1;
    let sent = wire.only_request();
    assert!(sent.url.starts_with("https://vertex.example/"));
    assert!(
        sent.url
            .contains("projects/pinned-project/locations/europe-west4")
    );
    assert!(sent.url.ends_with(&format!("/models/{upstream}{verb}")));
    assert_eq!(json(body_text(&sent)), json(expected_body));
    assert_eq!(result.response.model, upstream);
    assert_eq!(result.response.object, "list");
    assert!(!result.response.data.is_empty());
}

#[tokio::test]
async fn embed_content_usage_comes_from_the_reply() {
    let wire = Wire::answering(200, EMBED_CONTENT_ANSWER);
    let result = embed(
        &calls(&wire, PINNED),
        "vertex_ai/gemini-embedding-2",
        TEXT_REQUEST,
    )
    .await
    .expect("an embedding");
    assert_eq!(result.response.usage.prompt_tokens, 3);
    assert_eq!(result.response.usage.total_tokens, 4);
    assert_eq!(result.response.usage.completion_tokens, 0);
}

#[tokio::test]
async fn request_options_reach_the_text_body_through_the_vertex_mapping() {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let request =
        r#"{"model": "embed", "input": "hello", "dimensions": 8, "input_type": "RETRIEVAL_QUERY"}"#;
    embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        request,
    )
    .await
    .expect("an embedding");
    let body = wire.only_request();
    assert!(body_text(&body).contains(r#""outputDimensionality":8"#));
    assert!(body_text(&body).contains(r#""task_type":"RETRIEVAL_QUERY""#));
}

#[rstest]
#[case::text("vertex_ai/text-embedding-005", 429)]
#[case::embed_content("vertex_ai/gemini-embedding-2", 403)]
#[tokio::test]
async fn an_upstream_refusal_keeps_its_status_and_body(
    #[case] litellm_model: &str,
    #[case] status: u16,
) {
    let wire = Wire::answering(status, "denied");
    let Err(EmbeddingFailure::Route(EmbeddingError::Provider { response })) =
        embed(&calls(&wire, PINNED), litellm_model, TEXT_REQUEST).await
    else {
        panic!("a refused call keeps the upstream answer");
    };
    assert_eq!(response.status, status);
    assert_eq!(response.body, b"denied");
}

#[tokio::test]
async fn an_unreadable_answer_is_an_internal_failure() {
    let wire = Wire::answering(200, "not json");
    let outcome = embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        TEXT_REQUEST,
    )
    .await;
    assert!(matches!(
        outcome.err(),
        Some(EmbeddingFailure::Internal { .. })
    ));
}

#[rstest]
#[case::vertex_credentials("vertex_credentials")]
#[case::vertex_ai_credentials("vertex_ai_credentials")]
#[tokio::test]
async fn a_request_cannot_choose_the_credentials(#[case] name: &str) {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let request = format!(r#"{{"model": "embed", "input": "hello", "{name}": "creds.json"}}"#);
    let outcome = embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        &request,
    )
    .await;
    let Some(EmbeddingFailure::Route(EmbeddingError::Unsupported { name: refused })) =
        outcome.err()
    else {
        panic!("a request that names credentials is refused");
    };
    assert_eq!(refused, name);
    assert!(wire.sent.lock().expect("the call log").is_empty());
}

#[rstest]
#[case::base64(r#""encoding_format": "base64""#, "encoding_format")]
#[case::user(r#""user": "someone""#, "user")]
#[tokio::test]
async fn unsupported_openai_options_are_refused_by_name(
    #[case] option: &str,
    #[case] expected: &str,
) {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let request = format!(r#"{{"model": "embed", "input": "hello", {option}}}"#);
    let outcome = embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        &request,
    )
    .await;
    let Some(EmbeddingFailure::Route(EmbeddingError::Unsupported { name })) = outcome.err() else {
        panic!("the option is refused as unsupported");
    };
    assert_eq!(name, expected);
    assert!(wire.sent.lock().expect("the call log").is_empty());
}

#[tokio::test]
async fn float_encoding_is_a_no_op() {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let request = r#"{"model": "embed", "input": "hello", "encoding_format": "float"}"#;
    embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        request,
    )
    .await
    .expect("an embedding");
    assert_eq!(
        body_text(&wire.only_request()),
        r#"{"instances":[{"content":"hello"}],"parameters":{}}"#
    );
}

#[tokio::test]
async fn token_input_is_refused_for_embed_content() {
    let wire = Wire::answering(200, EMBED_CONTENT_ANSWER);
    let outcome = embed(
        &calls(&wire, PINNED),
        "vertex_ai/gemini-embedding-2",
        r#"{"model": "embed", "input": [1, 2]}"#,
    )
    .await;
    let Some(EmbeddingFailure::Route(EmbeddingError::Unsupported { name })) = outcome.err() else {
        panic!("token input has no embedContent form");
    };
    assert_eq!(name, "input");
}

#[tokio::test]
async fn a_wrong_typed_routing_option_is_an_internal_failure() {
    let wire = Wire::answering(200, TEXT_ANSWER);
    let outcome = embed(
        &calls(&wire, PINNED),
        "vertex_ai/text-embedding-005",
        r#"{"model": "embed", "input": "hello", "vertex_project": 5}"#,
    )
    .await;
    let Some(EmbeddingFailure::Internal { message }) = outcome.err() else {
        panic!("a numeric project is not a routing option");
    };
    assert_eq!(message, "the vertex_project option must be a string");
}
