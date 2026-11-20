#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::assistant_settings::{Config, Settings};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_core::api::assistant_run_sdk::service::RunOutcome;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::assistants::transport::AssistantsTransport;
use betterlite_openai::thread_runs::error::ThreadRunsError;
use betterlite_openai::thread_runs::stream::{RunChunks, RunStream, RunStreamTransport};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use secrecy::SecretString;

use super::{
    AssistantBodyError, AssistantClient, AssistantDispatchError, AssistantDispatcher,
    AssistantOperation, AssistantOutcome, AssistantRequest, SuppliedClass, decode_assistant_body,
};

const ASSISTANT: &str = r#"{"id":"asst_a","object":"assistant","model":"gpt-4o-mini","created_at":1,"name":null,"tools":[]}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"asst_a","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}],"has_more":false,"first_id":"asst_a","last_id":"asst_a"}"#;
const DELETED: &str = r#"{"id":"asst_a","object":"assistant.deleted","deleted":true}"#;
const THREAD: &str =
    r#"{"id":"thread_a","object":"thread","created_at":1,"metadata":{},"tool_resources":{}}"#;
const MESSAGE: &str = r#"{"id":"msg_a","object":"thread.message","thread_id":"thread_a","role":"user","content":[],"created_at":1}"#;
const MESSAGE_PAGE: &str = r#"{"object":"list","data":[{"id":"msg_a","object":"thread.message","thread_id":"thread_a","role":"user","content":[],"created_at":1}],"has_more":false,"first_id":"msg_a","last_id":"msg_a"}"#;
const RUN: &str = r#"{"id":"run_a","object":"thread.run","created_at":1,"assistant_id":"asst_a","thread_id":"thread_a","status":"queued","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
const MOCK_BASE: &str = "https://mock.test/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    url: String,
    authorization: Option<String>,
    api_key: Option<String>,
    body: Json,
}

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<Seen>>,
    replies: Mutex<VecDeque<(u16, String)>>,
}

impl Recorder {
    fn queue(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back((status, body.to_owned()));
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next(&self) -> (u16, String) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or((500, String::from("{}")))
    }
}

fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn body_of(body: &RequestBody) -> Json {
    match body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Json::Null),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Json::Null
        }
    }
}

impl AssistantsTransport for Recorder {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                method: request.method.as_str().to_owned(),
                url: url.to_string(),
                authorization: header(&request, "authorization"),
                api_key: header(&request, "api-key"),
                body: body_of(&request.body),
            });
        let (status, body) = self.next();
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

impl RunStreamTransport for Recorder {
    fn open(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<RunChunks, ThreadRunsError>> + Send {
        ready(Err(ThreadRunsError::Status(500, Vec::new())))
    }
}

struct OutcomeView {
    result: Result<AssistantOutcome<RunStream<Recorder>>, AssistantDispatchError>,
    seen: Vec<Seen>,
}

async fn dispatch(
    settings: &Settings,
    operation: AssistantOperation,
    data: BTreeMap<String, Json>,
    thread_id: Option<&str>,
    assistant_id: Option<&str>,
    client: Option<AssistantClient>,
    replies: &[(u16, &str)],
) -> OutcomeView {
    let recorder = Arc::new(Recorder::default());
    for (status, body) in replies {
        recorder.queue(*status, body);
    }
    let env = BTreeMap::new();
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("{}")
    .expect("an empty config loads");
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor starts");
    let dispatcher = AssistantDispatcher {
        settings,
        resources: &loaded.sections.resource_sdk_settings,
        azure: &loaded.sections.azure_endpoints,
        environment: &env,
        sources: NoSources,
        resolution: &resolution,
        redactor: &redactor,
        transport: Arc::clone(&recorder),
        limits: ApiLimits::default(),
    };
    let request = AssistantRequest {
        operation,
        data: &data,
        thread_id,
        assistant_id,
        client,
        event_handler: None,
    };
    let result = dispatcher.dispatch(&request).await;
    let seen = recorder.seen();
    OutcomeView { result, seen }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn data(fields: &[(&str, Json)]) -> BTreeMap<String, Json> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn configured(provider: &str, params: &[(&str, Json)]) -> Settings {
    Settings {
        config: Some(Config {
            custom_llm_provider: Some(provider.to_owned()),
            litellm_params: Some(data(params)),
        }),
    }
}

fn supplied(api_key: &str) -> AssistantClient {
    AssistantClient {
        class: SuppliedClass::AsyncOpenAi,
        api_key: Some(SecretString::from(api_key.to_owned())),
        api_base: Some(MOCK_BASE.to_owned()),
        api_version: None,
        timeout: None,
        max_retries: None,
        organization: None,
    }
}

fn openai_data() -> BTreeMap<String, Json> {
    data(&[("api_base", text(MOCK_BASE)), ("api_key", text("sk-test"))])
}

#[tokio::test]
async fn dispatch_all_assistant_operations() {
    let settings = configured("openai", &[]);
    let created = dispatch(
        &settings,
        AssistantOperation::CreateAssistant,
        data(&[
            ("model", text("gpt-4o-mini")),
            ("name", text("helper")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        None,
        None,
        None,
        &[(200, ASSISTANT)],
    )
    .await;
    let AssistantOutcome::Assistant(assistant) = created.result.expect("create succeeds") else {
        panic!("create answered with another outcome");
    };
    assert_eq!(assistant.id, "asst_a");
    assert_eq!(created.seen[0].method, "POST");
    assert_eq!(created.seen[0].url, "https://mock.test/v1/assistants");
    assert_eq!(
        created.seen[0].authorization.as_deref(),
        Some("Bearer sk-test")
    );

    let listed = dispatch(
        &settings,
        AssistantOperation::ListAssistants,
        openai_data(),
        None,
        None,
        Some(supplied("sk-supplied")),
        &[(200, PAGE)],
    )
    .await;
    let AssistantOutcome::Assistants(page) = listed.result.expect("list succeeds") else {
        panic!("list answered with another outcome");
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(listed.seen[0].method, "GET");
    // a supplied client's own key reaches the SDK owner unchanged
    assert_eq!(
        listed.seen[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );

    let deleted = dispatch(
        &settings,
        AssistantOperation::DeleteAssistant,
        openai_data(),
        None,
        Some("asst_a"),
        None,
        &[(200, DELETED)],
    )
    .await;
    let AssistantOutcome::Deleted(removed) = deleted.result.expect("delete succeeds") else {
        panic!("delete answered with another outcome");
    };
    assert!(removed.deleted);
    assert_eq!(deleted.seen[0].method, "DELETE");
    assert_eq!(
        deleted.seen[0].url,
        "https://mock.test/v1/assistants/asst_a"
    );

    let thread = dispatch(
        &settings,
        AssistantOperation::CreateThread,
        openai_data(),
        None,
        None,
        None,
        &[(200, THREAD)],
    )
    .await;
    let AssistantOutcome::Thread(Some(record)) = thread.result.expect("create thread succeeds")
    else {
        panic!("create thread answered with another outcome");
    };
    assert_eq!(record.id, "thread_a");
    assert_eq!(thread.seen[0].method, "POST");
    assert_eq!(thread.seen[0].url, "https://mock.test/v1/threads");

    let fetched = dispatch(
        &settings,
        AssistantOperation::GetThread,
        openai_data(),
        Some("thread_a"),
        None,
        None,
        &[(200, THREAD)],
    )
    .await;
    let AssistantOutcome::Thread(Some(fetched_record)) =
        fetched.result.expect("get thread succeeds")
    else {
        panic!("get thread answered with another outcome");
    };
    assert_eq!(fetched_record.id, "thread_a");
    assert_eq!(fetched.seen[0].url, "https://mock.test/v1/threads/thread_a");

    let message = dispatch(
        &settings,
        AssistantOperation::CreateMessage,
        data(&[
            ("role", text("user")),
            ("content", text("hello")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Some("thread_a"),
        None,
        None,
        &[(200, MESSAGE)],
    )
    .await;
    let AssistantOutcome::Message(added) = message.result.expect("add message succeeds") else {
        panic!("add message answered with another outcome");
    };
    assert_eq!(added.id, "msg_a");
    assert_eq!(message.seen[0].method, "POST");
    assert_eq!(
        message.seen[0].url,
        "https://mock.test/v1/threads/thread_a/messages"
    );

    let messages = dispatch(
        &settings,
        AssistantOperation::ListMessages,
        openai_data(),
        Some("thread_a"),
        None,
        None,
        &[(200, MESSAGE_PAGE)],
    )
    .await;
    let AssistantOutcome::Messages(page) = messages.result.expect("list messages succeeds") else {
        panic!("list messages answered with another outcome");
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(messages.seen[0].method, "GET");

    let run = dispatch(
        &settings,
        AssistantOperation::RunThread,
        data(&[
            ("assistant_id", text("asst_a")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Some("thread_a"),
        None,
        None,
        &[(200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Polled(reply)) = run.result.expect("run succeeds") else {
        panic!("run answered with another outcome");
    };
    assert_eq!(reply.expect("the provider replied").id, "run_a");
    assert_eq!(run.seen[0].method, "POST");
    assert_eq!(
        run.seen[0].url,
        "https://mock.test/v1/threads/thread_a/runs"
    );

    // stream=true stays a lazy manager until its owner enters it; no provider request has happened yet
    let streamed = dispatch(
        &settings,
        AssistantOperation::RunThread,
        data(&[
            ("assistant_id", text("asst_a")),
            ("stream", Json::Bool(true)),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Some("thread_a"),
        None,
        None,
        &[(200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Streaming(_manager)) =
        streamed.result.expect("run stream succeeds")
    else {
        panic!("run stream answered with another outcome");
    };
    assert!(streamed.seen.is_empty());
}

#[tokio::test]
async fn assistant_config_overlays_request() {
    let settings = configured(
        "openai",
        &[
            ("api_key", text("config-key")),
            ("model", text("gpt-4o-mini")),
        ],
    );
    let original = settings.clone();
    let overlaid = dispatch(
        &settings,
        AssistantOperation::CreateAssistant,
        data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
        None,
        None,
        None,
        &[(200, ASSISTANT)],
    )
    .await;
    overlaid.result.expect("the configured call succeeds");
    assert_eq!(
        overlaid.seen[0].authorization.as_deref(),
        Some("Bearer config-key")
    );
    let body = match &overlaid.seen[0].body {
        Json::Object(fields) => fields.clone(),
        other => panic!("the provider body is not an object: {other:?}"),
    };
    assert_eq!(body.get("model"), Some(&text("gpt-4o-mini")));
    assert_eq!(settings, original, "the configured map stays unchanged");
    let params = original
        .config
        .expect("configured")
        .litellm_params
        .expect("parameters");
    assert_eq!(params.get("api_key"), Some(&text("config-key")));

    // an explicit provider bypasses the config overlay, key and model included
    let explicit = dispatch(
        &settings,
        AssistantOperation::CreateAssistant,
        data(&[
            ("custom_llm_provider", text("azure")),
            ("api_key", text("request-key")),
            ("api_base", text(MOCK_BASE)),
            ("api_version", text("2024-05-01-preview")),
            ("model", text("gpt-4o")),
        ]),
        None,
        None,
        None,
        &[(200, ASSISTANT)],
    )
    .await;
    explicit.result.expect("the explicit azure call succeeds");
    assert_eq!(explicit.seen[0].api_key.as_deref(), Some("request-key"));
    match &explicit.seen[0].body {
        Json::Object(fields) => assert_eq!(fields.get("model"), Some(&text("gpt-4o"))),
        other => panic!("the provider body is not an object: {other:?}"),
    }

    // an empty provider is a value, so the config is not read and the SDK rejects it
    let empty = dispatch(
        &settings,
        AssistantOperation::CreateAssistant,
        data(&[
            ("custom_llm_provider", text("")),
            ("api_key", text("request-key")),
            ("model", text("gpt-4o")),
        ]),
        None,
        None,
        None,
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Failure(failure)) = empty.result else {
        panic!("the empty provider did not surface as a failure");
    };
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, Some(400));
    assert!(failure.message.contains("doesn't support"), "{failure}");
    assert!(empty.seen.is_empty());

    // a configured client or provider collides with the installed function's own named arguments
    for name in ["client", "custom_llm_provider"] {
        let colliding = configured("openai", &[(name, text("configured"))]);
        let outcome = dispatch(
            &colliding,
            AssistantOperation::CreateAssistant,
            data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
            None,
            None,
            None,
            &[],
        )
        .await;
        let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
            panic!("the configured keyword did not collide");
        };
        assert_eq!(
            message,
            format!("acreate_assistants() got multiple values for keyword argument '{name}'")
        );
        assert!(outcome.seen.is_empty());
    }
}

#[tokio::test]
async fn assistant_dispatch_requires_provider() {
    let absent = dispatch(
        &Settings::default(),
        AssistantOperation::CreateAssistant,
        data(&[("model", text("gpt-4o-mini")), ("api_key", text("sk-test"))]),
        None,
        None,
        None,
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Exception { message }) = absent.result else {
        panic!("no provider did not raise the factory exception");
    };
    assert_eq!(
        message,
        "'custom_llm_provider' must be set. Either via:\n `Router(assistants_config={'custom_llm_provider': ..})` \nor\n `router.arun_thread(custom_llm_provider=..)`"
    );
    assert!(absent.seen.is_empty());

    let empty_config = Settings {
        config: Some(Config {
            custom_llm_provider: None,
            litellm_params: Some(BTreeMap::new()),
        }),
    };
    let keyed = dispatch(
        &empty_config,
        AssistantOperation::CreateAssistant,
        data(&[("model", text("gpt-4o-mini"))]),
        None,
        None,
        None,
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = keyed.result else {
        panic!("an empty config did not raise");
    };
    assert_eq!(key, "custom_llm_provider");
    assert!(keyed.seen.is_empty());

    let only_provider = Settings {
        config: Some(Config {
            custom_llm_provider: Some(String::from("openai")),
            litellm_params: None,
        }),
    };
    let missing_params = dispatch(
        &only_provider,
        AssistantOperation::CreateAssistant,
        data(&[("model", text("gpt-4o-mini"))]),
        None,
        None,
        None,
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = missing_params.result else {
        panic!("the missing parameters did not raise");
    };
    assert_eq!(key, "litellm_params");
    assert!(missing_params.seen.is_empty());

    let rejected = dispatch(
        &Settings::default(),
        AssistantOperation::CreateAssistant,
        data(&[
            ("custom_llm_provider", text("anthropic")),
            ("model", text("claude-3")),
        ]),
        None,
        None,
        None,
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Failure(failure)) = rejected.result else {
        panic!("the unsupported provider did not surface as a failure");
    };
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, Some(400));
    assert!(failure.message.contains("'create_assistants'"), "{failure}");
    assert!(rejected.seen.is_empty());
}

#[test]
fn assistant_body_is_strict() {
    let limits = ApiLimits::default();
    // a duplicate key keeps the last value, exactly as orjson.loads does
    let duplicate = decode_assistant_body(br#"{"model":"first","model":"last"}"#, &limits)
        .expect("a duplicate key parses");
    match duplicate {
        Json::Object(fields) => assert_eq!(fields.get("model"), Some(&text("last"))),
        other => panic!("the object parsed as {other:?}"),
    }
    // null and arrays stay exactly what the caller wrote for the route to reject
    assert_eq!(
        decode_assistant_body(b"null", &limits).expect("null parses"),
        Json::Null
    );
    assert!(matches!(
        decode_assistant_body(b"[1,2]", &limits).expect("an array parses"),
        Json::List(_)
    ));
    // empty, truncated, invalid UTF-8 and NaN are decode errors with no repair
    for malformed in [
        b"".as_slice(),
        b"{",
        b"{\"model\": }",
        &[0xff_u8, 0xfe, b'{'],
        b"{\"model\": NaN}",
        b"{'model': 'x'}",
        b"{\"model\": \"x\",}",
    ] {
        assert!(
            decode_assistant_body(malformed, &limits).is_err(),
            "the bytes {malformed:?} are not JSON"
        );
    }
    // B1087's byte boundary is the decoder's own failure
    let over = vec![b'a'; limits.max_json_bytes.saturating_add(1)];
    let Err(AssistantBodyError::Limits(error)) = decode_assistant_body(&over, &limits) else {
        panic!("the oversized body decoded");
    };
    let fields = error.fields();
    assert_eq!(fields.status, 413);
    assert_eq!(fields.error_type, "invalid_request_error");
}

#[test]
fn replays_corpus() {
    let seeds =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/assistant_body");
    let replayed = std::fs::read_dir(seeds)
        .expect("the corpus directory exists")
        .map(|entry| {
            let seed =
                std::fs::read(entry.expect("an entry is readable").path()).expect("a seed reads");
            let _decoded = decode_assistant_body(&seed, &ApiLimits::default());
        })
        .count();
    assert!(replayed >= 6);
}
