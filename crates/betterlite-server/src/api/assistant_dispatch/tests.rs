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

async fn create_assistant_answers_with_the_record(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[
            ("model", text("gpt-4o-mini")),
            ("name", text("helper")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route::default(),
        &[(200, ASSISTANT)],
    )
    .await;
    let AssistantOutcome::Assistant(assistant) = outcome.result.expect("create succeeds") else {
        panic!("create answered with another outcome");
    };
    assert_eq!(assistant.id, "asst_a");
    assert_eq!(outcome.seen[0].method, "POST");
    assert_eq!(outcome.seen[0].url, "https://mock.test/v1/assistants");
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-test")
    );
}

async fn list_assistants_uses_the_supplied_client(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::ListAssistants,
        &openai_data(),
        Route {
            client: Some(supplied("sk-supplied")),
            ..Route::default()
        },
        &[(200, PAGE)],
    )
    .await;
    let AssistantOutcome::Assistants(page) = outcome.result.expect("list succeeds") else {
        panic!("list answered with another outcome");
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(outcome.seen[0].method, "GET");
    // a supplied client's own key reaches the SDK owner unchanged
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn delete_assistant_passes_the_path_id(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &openai_data(),
        Route {
            assistant_id: Some("asst_a"),
            ..Route::default()
        },
        &[(200, DELETED)],
    )
    .await;
    let AssistantOutcome::Deleted(removed) = outcome.result.expect("delete succeeds") else {
        panic!("delete answered with another outcome");
    };
    assert!(removed.deleted);
    assert_eq!(outcome.seen[0].method, "DELETE");
    assert_eq!(
        outcome.seen[0].url,
        "https://mock.test/v1/assistants/asst_a"
    );
}

async fn create_thread_posts_the_body(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateThread,
        &openai_data(),
        Route::default(),
        &[(200, THREAD)],
    )
    .await;
    let AssistantOutcome::Thread(Some(record)) = outcome.result.expect("create thread succeeds")
    else {
        panic!("create thread answered with another outcome");
    };
    assert_eq!(record.id, "thread_a");
    assert_eq!(outcome.seen[0].method, "POST");
    assert_eq!(outcome.seen[0].url, "https://mock.test/v1/threads");
}

async fn get_thread_uses_the_supplied_client(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::GetThread,
        &openai_data(),
        Route {
            thread_id: Some("thread_a"),
            client: Some(supplied("sk-supplied")),
            ..Route::default()
        },
        &[(200, THREAD)],
    )
    .await;
    let AssistantOutcome::Thread(Some(record)) = outcome.result.expect("get thread succeeds")
    else {
        panic!("get thread answered with another outcome");
    };
    assert_eq!(record.id, "thread_a");
    assert_eq!(outcome.seen[0].url, "https://mock.test/v1/threads/thread_a");
    // a supplied client's own key reaches the thread facade unchanged
    assert_eq!(
        outcome.seen[0].authorization.as_deref(),
        Some("Bearer sk-supplied")
    );
}

async fn create_message_posts_to_the_thread(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateMessage,
        &data(&[
            ("role", text("user")),
            ("content", text("hello")),
            // a body client key is not forwarded to the wrapper, so it never binds the client argument
            ("client", object(&[("api_key", text("sk-body"))])),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[(200, MESSAGE)],
    )
    .await;
    let AssistantOutcome::Message(added) = outcome.result.expect("add message succeeds") else {
        panic!("add message answered with another outcome");
    };
    assert_eq!(added.id, "msg_a");
    assert_eq!(outcome.seen[0].method, "POST");
    assert_eq!(
        outcome.seen[0].url,
        "https://mock.test/v1/threads/thread_a/messages"
    );
}

async fn list_messages_reads_the_thread(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::ListMessages,
        &openai_data(),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[(200, MESSAGE_PAGE)],
    )
    .await;
    let AssistantOutcome::Messages(page) = outcome.result.expect("list messages succeeds") else {
        panic!("list messages answered with another outcome");
    };
    assert_eq!(page.data.len(), 1);
    assert_eq!(outcome.seen[0].method, "GET");
}

async fn a_polled_run_returns_the_reply(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::RunThread,
        &data(&[
            ("assistant_id", text("asst_a")),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[(200, RUN), (200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Polled(reply)) = outcome.result.expect("run succeeds")
    else {
        panic!("run answered with another outcome");
    };
    assert_eq!((*reply).expect("the provider replied").id, "run_a");
    assert_eq!(outcome.seen[0].method, "POST");
    assert_eq!(
        outcome.seen[0].url,
        "https://mock.test/v1/threads/thread_a/runs"
    );
}

async fn a_streamed_run_stays_lazy(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::RunThread,
        &data(&[
            ("assistant_id", text("asst_a")),
            ("stream", Json::Bool(true)),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[(200, RUN)],
    )
    .await;
    let AssistantOutcome::Run(RunOutcome::Streaming(_)) =
        outcome.result.expect("run stream succeeds")
    else {
        panic!("run stream answered with another outcome");
    };
    // stream=true stays a lazy manager until its owner enters it; no provider request has happened yet
    assert!(outcome.seen.is_empty());
}

#[tokio::test]
async fn dispatch_all_assistant_operations() {
    let settings = configured("openai", &[]);
    create_assistant_answers_with_the_record(&settings).await;
    list_assistants_uses_the_supplied_client(&settings).await;
    delete_assistant_passes_the_path_id(&settings).await;
    create_thread_posts_the_body(&settings).await;
    get_thread_uses_the_supplied_client(&settings).await;
    create_message_posts_to_the_thread(&settings).await;
    list_messages_reads_the_thread(&settings).await;
    a_polled_run_returns_the_reply(&settings).await;
    a_streamed_run_stays_lazy(&settings).await;
}

// arun_thread reads a supplied client and its optional kwargs through the run owner
#[tokio::test]
async fn a_supplied_client_reaches_the_run_owner() {
    let settings = configured("openai", &[]);
    let fixtures = fixture(&settings);
    let metadata = object(&[("suite", text("b2489"))]);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::RunThread,
        &data(&[
            ("assistant_id", text("asst_a")),
            ("metadata", metadata.clone()),
            ("api_base", text(MOCK_BASE)),
            ("api_key", text("sk-test")),
        ]),
        Route {
            thread_id: Some("thread_a"),
            client: Some(supplied("sk-supplied")),
            ..Route::default()
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
            assistant_id: Some("asst_a"),
            ..Route::default()
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
            assistant_id: Some("asst_a"),
            ..Route::default()
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

async fn an_empty_configured_provider_raises_key_error() {
    let empty_config = Settings {
        config: Some(Config {
            custom_llm_provider: None,
            litellm_params: Some(BTreeMap::new()),
        }),
    };
    let fixtures = fixture(&empty_config);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = outcome.result else {
        panic!("an empty config did not raise");
    };
    assert_eq!(key, "custom_llm_provider");
    assert!(outcome.seen.is_empty());
}

async fn a_configured_provider_without_parameters_raises_key_error() {
    let only_provider = Settings {
        config: Some(Config {
            custom_llm_provider: Some(String::from("openai")),
            litellm_params: None,
        }),
    };
    let fixtures = fixture(&only_provider);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateAssistant,
        &data(&[("model", text("gpt-4o-mini"))]),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::KeyError { key }) = outcome.result else {
        panic!("the missing parameters did not raise");
    };
    assert_eq!(key, "litellm_params");
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
    an_empty_configured_provider_raises_key_error().await;
    a_configured_provider_without_parameters_raises_key_error().await;
    an_unsupported_provider_surfaces_as_a_bad_request().await;
}

async fn missing_role_and_content_are_a_paired_error(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateMessage,
        &openai_data(),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing role and content did not raise");
    };
    assert_eq!(
        message,
        "a_add_message() missing 2 required positional arguments: 'role' and 'content'"
    );
    assert!(outcome.seen.is_empty());
}

async fn a_missing_assistant_id_is_a_single_error(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::RunThread,
        &openai_data(),
        Route {
            thread_id: Some("thread_a"),
            ..Route::default()
        },
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing assistant id did not raise");
    };
    assert_eq!(
        message,
        "arun_thread() missing 1 required positional argument: 'assistant_id'"
    );
    assert!(outcome.seen.is_empty());
}

// the route always carries the path id, so only a caller that builds the request itself reaches this
async fn a_delete_without_a_path_id_is_a_single_error(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::DeleteAssistant,
        &openai_data(),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing assistant id did not raise");
    };
    assert_eq!(
        message,
        "delete_assistant() missing 1 required positional argument: 'assistant_id'"
    );
    assert!(outcome.seen.is_empty());
}

async fn a_missing_thread_id_lists_every_argument(settings: &Settings) {
    let fixtures = fixture(settings);
    let outcome = dispatch(
        &fixtures,
        AssistantOperation::CreateMessage,
        &openai_data(),
        Route::default(),
        &[],
    )
    .await;
    let Err(AssistantDispatchError::TypeError { message }) = outcome.result else {
        panic!("the missing thread id did not raise");
    };
    assert_eq!(
        message,
        "a_add_message() missing 3 required positional arguments: 'thread_id', 'role', and 'content'"
    );
    assert!(outcome.seen.is_empty());
}

#[tokio::test]
async fn missing_arguments_use_the_interpreter_wording() {
    let settings = configured("openai", &[]);
    missing_role_and_content_are_a_paired_error(&settings).await;
    a_missing_assistant_id_is_a_single_error(&settings).await;
    a_delete_without_a_path_id_is_a_single_error(&settings).await;
    a_missing_thread_id_lists_every_argument(&settings).await;
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
