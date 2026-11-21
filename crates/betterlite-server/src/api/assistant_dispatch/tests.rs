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

use super::AssistantDispatcher;
use super::body::{AssistantBodyError, decode_assistant_body};
use super::client::{AssistantClient, SuppliedClass};
use super::error::AssistantDispatchError;
use super::operation::AssistantOperation;
use super::outcome::AssistantOutcome;
use super::request::AssistantRequest;

mod dispatch;
mod missing_arguments;
mod operations;
mod supplied;

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

// the constructor identity each SDK owner receives, with the class the caller built
fn supplied(
    class: SuppliedClass,
    api_key: &str,
    api_base: &str,
    api_version: Option<&str>,
) -> AssistantClient {
    AssistantClient {
        class,
        api_key: Some(SecretString::from(api_key.to_owned())),
        api_base: Some(api_base.to_owned()),
        api_version: api_version.map(str::to_owned),
        timeout: None,
        max_retries: None,
        organization: None,
    }
}

fn openai_data() -> BTreeMap<String, Json> {
    data(&[("api_base", text(MOCK_BASE)), ("api_key", text("sk-test"))])
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
    // a float that fits stays a float, and an integer too large for a double stays an integer
    let finite = decode_assistant_body(
        b"{\"temperature\": 1e308, \"seed\": 123456789012345678901234567890}",
        &limits,
    )
    .expect("finite numbers parse");
    let Json::Object(finite_fields) = finite else {
        panic!("the object did not parse as a mapping");
    };
    assert!(matches!(
        finite_fields.get("temperature"),
        Some(Json::Number(_))
    ));
    assert!(matches!(finite_fields.get("seed"), Some(Json::Number(_))));
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
#[rstest::rstest]
#[case::empty(b"")]
#[case::truncated(b"{")]
#[case::missing_value(b"{\"model\": }")]
#[case::invalid_utf8(&[0xff_u8, 0xfe, b'{'])]
#[case::nan(b"{\"model\": NaN}")]
#[case::infinite(b"{\"model\": 1e400}")]
#[case::negative_infinite(b"[-1e400]")]
#[case::infinite_nested(b"{\"metadata\": {\"temperature\": 1e309}}")]
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
