#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{read, read_dir};
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::assistant_settings::{Config, Settings};
use betterlite_config::sections::{azure_endpoints, resource_sdk_settings};
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
use rstest::rstest;
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
const RUN: &str = r#"{"id":"run_a","object":"thread.run","created_at":1,"assistant_id":"asst_a","thread_id":"thread_a","status":"completed","instructions":"base","model":"gpt-4o-mini","parallel_tool_calls":true,"tools":[]}"#;
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
            .unwrap_or_else(|| (500, String::from("{}")))
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

#[derive(Default)]
struct Route<'route> {
    thread_id: Option<&'route str>,
    assistant_id: Option<&'route str>,
    client: Option<AssistantClient>,
}

// The loaded config and the borrowed services one dispatch shares with the dispatcher.
struct Fixtures<'settings> {
    settings: &'settings Settings,
    environment: BTreeMap<String, String>,
    resources: resource_sdk_settings::Settings,
    azure: azure_endpoints::Settings,
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    redactor: Redactor,
}

fn fixture(settings: &Settings) -> Fixtures<'_> {
    let environment = BTreeMap::new();
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("{}")
    .expect("an empty config loads");
    Fixtures {
        settings,
        environment,
        resources: loaded.sections.resource_sdk_settings,
        azure: loaded.sections.azure_endpoints,
        catalog: ModelCatalog::default(),
        names: BTreeSet::new(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor starts"),
    }
}

impl Fixtures<'_> {
    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn request<'request>(
    operation: AssistantOperation,
    data: &'request BTreeMap<String, Json>,
    route: Route<'request>,
) -> AssistantRequest<'request> {
    AssistantRequest {
        operation,
        data,
        thread_id: route.thread_id,
        assistant_id: route.assistant_id,
        client: route.client,
        event_handler: None,
    }
}

async fn dispatch(
    fixtures: &Fixtures<'_>,
    operation: AssistantOperation,
    data: &BTreeMap<String, Json>,
    route: Route<'_>,
    replies: &[(u16, &str)],
) -> OutcomeView {
    let recorder = Arc::new(Recorder::default());
    for (status, body) in replies {
        recorder.queue(*status, body);
    }
    let context = fixtures.resolution();
    let dispatcher = AssistantDispatcher {
        settings: fixtures.settings,
        resources: &fixtures.resources,
        azure: &fixtures.azure,
        environment: &fixtures.environment,
        sources: NoSources,
        resolution: &context,
        redactor: &fixtures.redactor,
        transport: Arc::clone(&recorder),
        limits: ApiLimits::default(),
    };
    let assistant_request = request(operation, data, route);
    let result = dispatcher.dispatch(&assistant_request).await;
    let seen = recorder.seen();
    OutcomeView { result, seen }
}

// one dispatch whose fixtures live only for the call
async fn call(
    settings: &Settings,
    operation: AssistantOperation,
    fields: &BTreeMap<String, Json>,
    route: Route<'_>,
    replies: &[(u16, &str)],
) -> OutcomeView {
    let fixtures = fixture(settings);
    dispatch(&fixtures, operation, fields, route, replies).await
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

fn object(fields: &[(&str, Json)]) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
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

// the record each installed operation answers with
#[derive(Debug)]
enum Answered {
    Assistant,
    Assistants,
    Deleted,
    Thread,
    Message,
    Messages,
    Polled,
    Streaming,
}

// the request one operation sends, as far as its case checks it
struct Sent {
    method: &'static str,
    url: Option<&'static str>,
    authorization: Option<&'static str>,
}

// one installed operation, its body, its route and what the mock provider answers
struct OpCase {
    operation: AssistantOperation,
    fields: BTreeMap<String, Json>,
    route: Route<'static>,
    replies: &'static [(u16, &'static str)],
    sent: Option<Sent>,
    answered: Answered,
}

// every assistant wrapper the router installs, each reaching its provider method
#[rstest]
#[case::create_assistant(OpCase {
    operation: AssistantOperation::CreateAssistant,
    fields: data(&[
        ("model", text("gpt-4o-mini")),
        ("name", text("helper")),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route::default(),
    replies: &[(200, ASSISTANT)],
    sent: Some(Sent {
        method: "POST",
        url: Some("https://mock.test/v1/assistants"),
        authorization: Some("Bearer sk-test"),
    }),
    answered: Answered::Assistant,
})]
#[case::list_assistants(OpCase {
    operation: AssistantOperation::ListAssistants,
    fields: openai_data(),
    route: Route {
        thread_id: None,
        assistant_id: None,
        client: Some(supplied("sk-supplied")),
    },
    replies: &[(200, PAGE)],
    sent: Some(Sent {
        method: "GET",
        url: None,
        // a supplied client's own key reaches the SDK owner unchanged
        authorization: Some("Bearer sk-supplied"),
    }),
    answered: Answered::Assistants,
})]
#[case::delete_assistant(OpCase {
    operation: AssistantOperation::DeleteAssistant,
    fields: openai_data(),
    route: Route {
        thread_id: None,
        assistant_id: Some("asst_a"),
        client: None,
    },
    replies: &[(200, DELETED)],
    sent: Some(Sent {
        method: "DELETE",
        url: Some("https://mock.test/v1/assistants/asst_a"),
        authorization: None,
    }),
    answered: Answered::Deleted,
})]
#[case::create_thread(OpCase {
    operation: AssistantOperation::CreateThread,
    fields: openai_data(),
    route: Route::default(),
    replies: &[(200, THREAD)],
    sent: Some(Sent {
        method: "POST",
        url: Some("https://mock.test/v1/threads"),
        authorization: None,
    }),
    answered: Answered::Thread,
})]
#[case::get_thread(OpCase {
    operation: AssistantOperation::GetThread,
    fields: openai_data(),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: Some(supplied("sk-supplied")),
    },
    replies: &[(200, THREAD)],
    sent: Some(Sent {
        method: "GET",
        url: Some("https://mock.test/v1/threads/thread_a"),
        // a supplied client's own key reaches the thread facade unchanged
        authorization: Some("Bearer sk-supplied"),
    }),
    answered: Answered::Thread,
})]
#[case::create_message(OpCase {
    operation: AssistantOperation::CreateMessage,
    fields: data(&[
        ("role", text("user")),
        ("content", text("hello")),
        // a body client key is not forwarded to the wrapper, so it never binds the client argument
        ("client", object(&[("api_key", text("sk-body"))])),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, MESSAGE)],
    sent: Some(Sent {
        method: "POST",
        url: Some("https://mock.test/v1/threads/thread_a/messages"),
        authorization: None,
    }),
    answered: Answered::Message,
})]
#[case::list_messages(OpCase {
    operation: AssistantOperation::ListMessages,
    fields: openai_data(),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, MESSAGE_PAGE)],
    sent: Some(Sent {
        method: "GET",
        url: None,
        authorization: None,
    }),
    answered: Answered::Messages,
})]
#[case::polled_run(OpCase {
    operation: AssistantOperation::RunThread,
    fields: data(&[
        ("assistant_id", text("asst_a")),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, RUN), (200, RUN)],
    sent: Some(Sent {
        method: "POST",
        url: Some("https://mock.test/v1/threads/thread_a/runs"),
        authorization: None,
    }),
    answered: Answered::Polled,
})]
#[case::streamed_run(OpCase {
    operation: AssistantOperation::RunThread,
    fields: data(&[
        ("assistant_id", text("asst_a")),
        ("stream", Json::Bool(true)),
        ("api_base", text(MOCK_BASE)),
        ("api_key", text("sk-test")),
    ]),
    route: Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    replies: &[(200, RUN)],
    sent: None,
    answered: Answered::Streaming,
})]
#[tokio::test]
async fn dispatch_all_assistant_operations(#[case] case: OpCase) {
    let settings = configured("openai", &[]);
    let outcome = call(
        &settings,
        case.operation,
        &case.fields,
        case.route,
        case.replies,
    )
    .await;
    assert_sent(&outcome, case.sent.as_ref());
    assert_answered(outcome, case.answered);
}

// the request the operation sent, or none when the case expects the lazy stream
fn assert_sent(outcome: &OutcomeView, sent: Option<&Sent>) {
    let Some(sent) = sent else {
        // stream=true stays a lazy manager until its owner enters it; no provider request happened yet
        assert!(outcome.seen.is_empty());
        return;
    };
    let seen = outcome.seen.first().expect("the operation sent a request");
    assert_eq!(seen.method, sent.method);
    if let Some(url) = sent.url {
        assert_eq!(seen.url, url);
    }
    if let Some(authorization) = sent.authorization {
        assert_eq!(seen.authorization.as_deref(), Some(authorization));
    }
}

// the record the operation answered with
fn assert_answered(outcome: OutcomeView, answered: Answered) {
    match (answered, outcome.result.expect("the operation answers")) {
        (Answered::Assistant, AssistantOutcome::Assistant(record)) => {
            assert_eq!(record.id, "asst_a");
        }
        (Answered::Assistants, AssistantOutcome::Assistants(page)) => {
            assert_eq!(page.data.first().expect("a listed assistant").id, "asst_a");
        }
        (Answered::Deleted, AssistantOutcome::Deleted(record)) => assert!(record.deleted),
        (Answered::Thread, AssistantOutcome::Thread(Some(record))) => {
            assert_eq!(record.id, "thread_a");
        }
        (Answered::Message, AssistantOutcome::Message(record)) => assert_eq!(record.id, "msg_a"),
        (Answered::Messages, AssistantOutcome::Messages(page)) => {
            assert_eq!(page.data.first().expect("a listed message").id, "msg_a");
        }
        (Answered::Polled, AssistantOutcome::Run(RunOutcome::Polled(reply))) => {
            assert_eq!((*reply).expect("the provider replied").id, "run_a");
        }
        (Answered::Streaming, AssistantOutcome::Run(RunOutcome::Streaming(_))) => {}
        (other, _) => panic!("the operation answered with another outcome: {other:?}"),
    }
}

// arun_thread reads a supplied client and its optional kwargs through the run owner
#[tokio::test]
async fn a_supplied_client_reaches_the_run_owner() {
    let settings = configured("openai", &[]);
    let metadata = object(&[("suite", text("b2489"))]);
    let outcome = call(
        &settings,
        AssistantOperation::RunThread,
        &data(&[
            ("assistant_id", text("asst_a")),
            ("metadata", metadata.clone()),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            assistant_id: None,
            client: Some(supplied("sk-supplied")),
        },
        &[(200, RUN), (200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Polled(reply)) = outcome.result.expect("run succeeds")
    else {
        panic!("run answered with another outcome");
    };
    assert_eq!((*reply).expect("the provider replied").id, "run_a");
    // the supplied client's own key wins over the params key, unchanged
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("metadata"), Some(&metadata));
}

async fn a_configured_parameter_overlays_the_request(settings: &Settings) {
    let fixtures = fixture(settings);
    let original = settings.clone();
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
        Route::default(),
        &[(200, ASSISTANT)],
    )
    .await;
    outcome.result.expect("the configured call succeeds");
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer config-key")
    );
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("model"), Some(&text("gpt-4o-mini")));
    assert_eq!(*settings, original, "the configured map stays unchanged");
    let params = original
        .config
        .expect("configured")
        .litellm_params
        .expect("parameters");
    assert_eq!(params.get("api_key"), Some(&text("config-key")));
}

async fn an_explicit_provider_bypasses_the_config(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("custom_llm_provider", text("azure")),
            ("api_key", text("request-key")),
            ("api_base", text(MOCK_BASE)),
            ("api_version", text("2024-05-01-preview")),
            ("model", text("gpt-4o")),
        ]),
        Route::default(),
        &[(200, ASSISTANT)],
    )
    .await;
    outcome.result.expect("the explicit azure call succeeds");
    assert_eq!(outcome.seen[0].api_key.as_deref(), Some("request-key"));
    let Json::Object(fields) = &outcome.seen[0].body else {
        panic!("the provider body is not an object");
    };
    assert_eq!(fields.get("model"), Some(&text("gpt-4o")));
}

async fn an_empty_provider_is_rejected(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("custom_llm_provider", text("")),
            ("api_key", text("request-key")),
            ("model", text("gpt-4o")),
        ]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Failure { failure }) = outcome.result else {
        panic!("the empty provider did not surface as a failure");
    };
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, Some(400));
    assert!(failure.message.contains("doesn't support"), "{failure:?}");
    assert!(outcome.seen.is_empty());
}

async fn configured_parameters_replace_the_path_keyword() {
    let settings = configured("openai", &[("assistant_id", text("asst_configured"))]);
    let fixtures = fixture(&settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &openai_data(),
        Route {
            thread_id: None,
            assistant_id: Some("asst_a"),
            client: None,
        },
        &[(200, DELETED)],
    )
    .await;
    outcome.result.expect("the configured id call succeeds");
    assert_eq!(
        outcome.seen[0].url,
        "https://mock.test/v1/assistants/asst_configured"
    );
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
    a_configured_parameter_overlays_the_request(&settings).await;
    an_explicit_provider_bypasses_the_config(&settings).await;
    an_empty_provider_is_rejected(&settings).await;
    configured_parameters_replace_the_path_keyword().await;
}

// a configured client or provider collides with the installed function's own named arguments
#[rstest]
#[case::client("client")]
#[case::provider("custom_llm_provider")]
#[tokio::test]
async fn a_configured_keyword_collides_with_the_installed_function(#[case] name: &str) {
    let colliding = configured("openai", &[(name, text("configured"))]);
    let fixtures = fixture(&colliding);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("api_key", text("request-key")), ("model", text("gpt-4o"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the configured keyword did not collide");
    };
    assert_eq!(
        message,
        format!(
            "litellm.assistants.main.acreate_assistants() got multiple values for keyword argument '{name}'"
        )
    );
    assert!(outcome.seen.is_empty());
}

// the route passes the path id as a keyword beside **data, so a body copy of that key collides
#[tokio::test]
async fn a_body_key_collides_with_the_route_argument() {
    let settings = configured("openai", &[]);
    let fixtures = fixture(&settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &data(&[("assistant_id", text("asst_body"))]),
        Route {
            thread_id: None,
            assistant_id: Some("asst_a"),
            client: None,
        },
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the body assistant id did not collide");
    };
    assert_eq!(
        message,
        "litellm.router.Router.factory_function.<locals>.async_wrapper() got multiple values for keyword argument 'assistant_id'"
    );
    assert!(outcome.seen.is_empty());
}

async fn no_configured_provider_raises_the_factory_exception() {
    let absent = Settings::default();
    let fixtures = fixture(&absent);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini")), ("api_key", text("sk-test"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Exception { message }) = outcome.result else {
        panic!("no provider did not raise the factory exception");
    };
    assert_eq!(
        message,
        "'custom_llm_provider' must be set. Either via:\n `Router(assistants_config={'custom_llm_provider': ..})` \nor\n `router.arun_thread(custom_llm_provider=..)`"
    );
    assert!(outcome.seen.is_empty());
}

// assistants_config reads both keys as plain dictionary entries, so an absent one is a KeyError
async fn a_missing_configured_key_raises_key_error(
    provider: Option<&str>,
    parameters: Option<BTreeMap<String, Json>>,
    expected: &str,
) {
    let settings = Settings {
        config: Some(Config {
            custom_llm_provider: provider.map(String::from),
            litellm_params: parameters,
        }),
    };
    let outcome = call(
        &settings,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = outcome.result else {
        panic!("the missing configured key did not raise");
    };
    assert_eq!(key, expected);
    assert!(outcome.seen.is_empty());
}

async fn an_unsupported_provider_surfaces_as_a_bad_request() {
    let absent = Settings::default();
    let fixtures = fixture(&absent);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("custom_llm_provider", text("anthropic")),
            ("model", text("claude-3")),
        ]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::Failure { failure }) = outcome.result else {
        panic!("the unsupported provider did not surface as a failure");
    };
    assert_eq!(failure.class, "BadRequestError");
    assert_eq!(failure.status, Some(400));
    assert!(
        failure.message.contains("'create_assistants'"),
        "{failure:?}"
    );
    assert!(outcome.seen.is_empty());
}

#[tokio::test]
async fn assistant_dispatch_requires_provider() {
    no_configured_provider_raises_the_factory_exception().await;
    a_missing_configured_key_raises_key_error(None, Some(BTreeMap::new()), "custom_llm_provider")
        .await;
    a_missing_configured_key_raises_key_error(Some("openai"), None, "litellm_params").await;
    an_unsupported_provider_surfaces_as_a_bad_request().await;
}

// the wrapper's own named arguments: a route without the id leaves it for the interpreter to report
#[rstest]
#[case::role_and_content(
    AssistantOperation::CreateMessage,
    Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    "a_add_message() missing 2 required positional arguments: 'role' and 'content'"
)]
#[case::assistant_id(
    AssistantOperation::RunThread,
    Route {
        thread_id: Some("thread_a"),
        assistant_id: None,
        client: None,
    },
    "arun_thread() missing 1 required positional argument: 'assistant_id'"
)]
// the route always carries the path id, so only a caller that builds the request itself reaches this
#[case::delete_id(
    AssistantOperation::DeleteAssistant,
    Route::default(),
    "delete_assistant() missing 1 required positional argument: 'assistant_id'"
)]
#[case::thread_id(
    AssistantOperation::CreateMessage,
    Route::default(),
    "a_add_message() missing 3 required positional arguments: 'thread_id', 'role', and 'content'"
)]
#[tokio::test]
async fn missing_arguments_use_the_interpreter_wording(
    #[case] operation: AssistantOperation,
    #[case] route: Route<'static>,
    #[case] expected: &str,
) {
    let settings = configured("openai", &[]);
    let outcome = call(&settings, operation, &openai_data(), route, &[]).await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing argument did not raise");
    };
    assert_eq!(message, expected);
    assert!(outcome.seen.is_empty());
}

#[test]
fn assistant_body_is_strict() {
    let limits = ApiLimits::default();
    // a duplicate key keeps the last value, exactly as orjson.loads does
    let duplicate = decode_assistant_body(br#"{"model":"first","model":"last"}"#, &limits)
        .expect("a duplicate key parses");
    let Json::Object(fields) = duplicate else {
        panic!("the object did not parse as a mapping");
    };
    assert_eq!(fields.get("model"), Some(&text("last")));
    // null and arrays stay exactly what the caller wrote for the route to reject
    assert_eq!(
        decode_assistant_body(b"null", &limits).expect("null parses"),
        Json::Null
    );
    assert!(matches!(
        decode_assistant_body(b"[1,2]", &limits).expect("an array parses"),
        Json::List(_)
    ));
    // B1087's byte boundary is the decoder's own failure
    let over = vec![b'a'; limits.max_json_bytes.saturating_add(1)];
    let Err(AssistantBodyError::Limits(error)) = decode_assistant_body(&over, &limits) else {
        panic!("the oversized body decoded");
    };
    let limit_fields = error.fields();
    assert_eq!(limit_fields.status, 413);
    assert_eq!(limit_fields.error_type, "invalid_request_error");
}

// empty, truncated, invalid UTF-8 and NaN are decode errors with no repair
#[rstest]
#[case::empty(b"")]
#[case::truncated(b"{")]
#[case::missing_value(b"{\"model\": }")]
#[case::invalid_utf8(&[0xff_u8, 0xfe, b'{'])]
#[case::nan(b"{\"model\": NaN}")]
#[case::single_quotes(b"{'model': 'x'}")]
#[case::trailing_comma(b"{\"model\": \"x\",}")]
fn malformed_assistant_bodies_are_rejected(#[case] malformed: &[u8]) {
    let failure = decode_assistant_body(malformed, &ApiLimits::default())
        .expect_err("the bytes are not JSON");
    assert!(
        failure.to_string().contains("not JSON"),
        "the bytes {malformed:?} failed with {failure}"
    );
}

#[test]
fn replays_corpus() {
    let seeds = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/assistant_body");
    let replayed = read_dir(seeds)
        .expect("the corpus directory exists")
        .map(|entry| {
            let seed = read(entry.expect("an entry is readable").path()).expect("a seed reads");
            let _decoded = decode_assistant_body(&seed, &ApiLimits::default());
        })
        .count();
    assert!(replayed >= 6);
}
