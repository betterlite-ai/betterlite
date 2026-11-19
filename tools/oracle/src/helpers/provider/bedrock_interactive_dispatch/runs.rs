use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::aws_settings::AuthParams;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::passthrough::collector::StreamLogAssembler;
use betterlite_bedrock::passthrough::reply::{NativeReplyRequest, NativeReplyTransform};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::{RealtimeClient, StreamError};
use betterlite_bedrock::realtime_transport::error::{ProviderClass, RealtimeTransportError};
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_bedrock::realtime_transport::runtime::{RealtimeRuntime, RuntimeConfig};
use betterlite_bedrock::realtime_transport::session::RealtimeCall;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::request::{SearchOptionalParams, SearchQuery};
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, SessionInput, SuccessWork,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_openai::search::provider::SearchQuery as SearchInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_provider::bedrock_interactive_dispatch::http::{
    LeafReply, LeafRequest, LeafTransport,
};
use betterlite_provider::bedrock_interactive_dispatch::{
    passthrough, realtime_bridge, responses, search, vector_retrieve,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{body_json, empty_row, error_row, request_row};

const RESPONSES_HOST: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";
const GATEWAY: &str = "https://gateway.example.com/mcp";
const VECTOR_ID: &str = "KB123";
const PASSTHROUGH_MODEL: &str = "anthropic.claude-3";
const REALTIME_MODEL: &str = "amazon.nova-sonic-v1:0";
const ACCESS: &str = "AKIDBETTERLITEFIXTURE";
const SECRET: &str = "fixture/secret/key/+EXAMPLEKEY0000";
const CATALOG: &str = r#"{"openai.gpt-oss-120b": {"supported_endpoints": ["/v1/responses"]}}"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesRun {
    model: String,
    #[serde(default)]
    model_cost: ParamMap,
    input: Value,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassthroughRun {
    endpoint: String,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    region: Option<String>,
    status: u16,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    extra_body: Option<Value>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RealtimeRun {
    pending: String,
}

fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_225_600))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from(ACCESS.to_owned()),
        secret_access_key: SecretString::from(SECRET.to_owned()),
        session_token: None,
    }
}

fn token(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn catalog() -> ParamMap {
    serde_json::from_str(CATALOG).unwrap_or_default()
}

fn block_on<F: Future>(future: F) -> Result<F::Output, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    Ok(runtime.block_on(future))
}

fn reply(status: u16, body: &str) -> LeafReply {
    LeafReply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
        chunks: Vec::new(),
    }
}

struct Scripted {
    reply: LeafReply,
}

impl LeafTransport for Scripted {
    fn send(
        &self,
        _request: LeafRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LeafReply, TransportError>> + Send + '_>> {
        Box::pin(ready(Ok(self.reply.clone())))
    }
}

struct Fakes;

impl ReasoningCapabilities for Fakes {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn catalogue_declares_default(&mut self) -> bool {
        false
    }
}

impl ResponsesCapabilities for Fakes {
    fn model_info(&mut self, _model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(ModelInfo::Unavailable)
    }
}

struct MapFetcher;

impl RemoteMediaFetcher for MapFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: urls.first().cloned().unwrap_or_default(),
        }))
    }
}

struct ResponsesFixture {
    input: ResponsesInput,
    params: ResponsesParams,
    model_cost: ParamMap,
    bearer: Option<SecretString>,
    capabilities: Fakes,
}

impl ResponsesFixture {
    fn new(run: &ResponsesRun) -> Result<Self, Box<HelperError>> {
        let input = serde_json::from_value::<ResponsesInput>(run.input.clone())?;
        let params = serde_json::from_value::<ResponsesParams>(
            run.params.clone().unwrap_or_else(|| json!({})),
        )?;
        let model_cost = if run.model_cost.is_empty() {
            catalog()
        } else {
            run.model_cost.clone()
        };
        Ok(Self {
            input,
            params,
            model_cost,
            bearer: run.api_key.as_deref().map(token),
            capabilities: Fakes,
        })
    }

    fn operation<'call>(
        &'call mut self,
        model: &'call str,
    ) -> responses::Operation<'call, MapFetcher, Fakes> {
        responses::Operation {
            aws: responses::AwsRequest {
                credentials: credentials(),
                api_base: Some(RESPONSES_HOST),
                aws_region_name: Some("us-west-2"),
                aws_bedrock_runtime_endpoint: None,
                environment_endpoint: None,
                region_inputs: RegionInputs::default(),
                api_key: self.bearer.as_ref(),
                environment_token: None,
                headers: IndexMap::new(),
                signing_params: BTreeMap::new(),
                now: fixture_time(),
            },
            body: responses::BodyRequest {
                input: &self.input,
                params: &self.params,
                model_info: None,
            },
            model: Some(model),
            model_cost: &self.model_cost,
            fetcher: MapFetcher,
            capabilities: &mut self.capabilities,
            drop_params: DropParams {
                global: false,
                request: false,
            },
            bundled: BundledReasoningModels::default(),
        }
    }
}

pub(super) fn responses_row(run: &ResponsesRun) -> super::HelperResult {
    let mut fixture = ResponsesFixture::new(run)?;
    let transport = Scripted {
        reply: reply(run.status, &run.reply),
    };
    match block_on(fixture.operation(&run.model).run(&transport))? {
        Ok(outcome) => Ok(responses_success(&outcome)),
        Err(error) => Ok(responses_failure(&error)),
    }
}

fn responses_success(outcome: &responses::Outcome) -> Value {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row(
            "POST",
            &outcome.request.url,
            &outcome.request.headers,
            &outcome.request.body,
        ),
    );
    row.insert("status".to_owned(), json!(outcome.status));
    row.insert("reply".to_owned(), body_json(&outcome.body));
    Value::Object(row)
}

fn responses_failure(error: &responses::Error) -> Value {
    match error {
        responses::Error::Leaf(_) => error_row("leaf", None),
        responses::Error::UnsupportedModel { .. } => error_row("unsupported_model", None),
        responses::Error::Transport(_) => error_row("transport", None),
        responses::Error::Rejected { status, .. } => error_row("rejected", Some(*status)),
        responses::Error::Headers { .. } => error_row("headers", None),
    }
}

struct Selector;

impl ChatConfigSelection for Selector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(Some(ChatConfigTag::Anthropic))
    }
}

struct Transform {
    result: Json,
}

impl NativeReplyTransform for Transform {
    fn transform(&self, _request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
        Ok(self.result.clone())
    }
}

struct Assembler;

impl StreamLogAssembler for Assembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        (!chunks.is_empty()).then(|| json!({"id": "assembled"}))
    }
}

struct Logging;

impl PassthroughLogging for Logging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

struct PassthroughServices {
    selector: Selector,
    transform: Transform,
    assembler: Assembler,
}

impl PassthroughServices {
    fn plan<'call>(&'call self, run: &'call PassthroughRun) -> BedrockPassthroughPlan<'call> {
        BedrockPassthroughPlan {
            environment: PassthroughEnvironment {
                region_name: Some(run.region.clone().unwrap_or_else(|| "us-east-1".to_owned())),
                region: None,
                session_region: None,
                runtime_endpoint: None,
            },
            identity: PassthroughIdentity {
                credentials: Some(credentials()),
                bearer_token: None,
                now: fixture_time(),
            },
            selector: &self.selector,
            transform: &self.transform,
            assembler: &self.assembler,
            clock: CollectorClock {
                next_id: || uuid::Uuid::from_u128(7),
                now: || 1_767_225_600,
                instant: || StreamInstant {
                    seconds: 1_767_225_600,
                    timestamp: 1_767_225_600.0,
                },
            },
        }
    }
}

fn passthrough_target(endpoint: &str) -> PassthroughTarget<'_> {
    PassthroughTarget {
        model: PASSTHROUGH_MODEL,
        custom_llm_provider: "bedrock",
        endpoint,
    }
}

fn passthrough_url_request<'call>(
    params: &'call Object,
    endpoint: &'call str,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: passthrough_target(endpoint),
        request_query_params: None,
        litellm_params: params,
    }
}

fn passthrough_base(
    plan: &BedrockPassthroughPlan<'_>,
    request: &PassthroughUrlRequest<'_>,
) -> Option<(String, String)> {
    let (url, endpoint_url) = plan.get_complete_url(request).ok()?;
    Some((url.to_string(), endpoint_url))
}

fn passthrough_transport(run: &PassthroughRun) -> Result<Scripted, Box<HelperError>> {
    let mut scripted = reply(run.status, &run.reply);
    for chunk in &run.chunks {
        scripted.chunks.push(hex::decode(chunk).or_value_error()?);
    }
    Ok(Scripted { reply: scripted })
}

pub(super) fn passthrough_row(run: &PassthroughRun) -> super::HelperResult {
    let services = PassthroughServices {
        selector: Selector,
        transform: Transform { result: Json::Null },
        assembler: Assembler,
    };
    let plan = services.plan(run);
    let params = Object::new();
    let url_request = passthrough_url_request(&params, &run.endpoint);
    let Some((signed_base, endpoint_url)) = passthrough_base(&plan, &url_request) else {
        return Ok(error_row("leaf", None));
    };
    let data = run
        .data
        .clone()
        .map(serde_json::from_value::<Object>)
        .transpose()?;
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: data.as_ref(),
        api_base: &signed_base,
        model: Some(PASSTHROUGH_MODEL),
    };
    let mut logging = Logging;
    let operation = passthrough::Operation {
        plan,
        url_request,
        signing,
        headers: Headers::new(),
        logging: &mut logging,
    };
    let transport = passthrough_transport(run)?;
    match block_on(operation.run(&transport))? {
        Ok(outcome) => Ok(passthrough_success(&outcome, &endpoint_url)),
        Err(error) => Ok(passthrough_failure(&error)),
    }
}

fn passthrough_success(outcome: &passthrough::Outcome, endpoint_url: &str) -> Value {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row(
            "POST",
            &outcome.request.url,
            &outcome.request.headers,
            &outcome.request.body,
        ),
    );
    row.insert("endpoint_url".to_owned(), json!(endpoint_url));
    row.insert("streaming".to_owned(), json!(outcome.streaming));
    row.insert("status".to_owned(), json!(outcome.status));
    row.insert("logged".to_owned(), json!(outcome.logged.is_some()));
    Value::Object(row)
}

fn passthrough_failure(error: &passthrough::Error) -> Value {
    match error {
        passthrough::Error::Leaf(_) => error_row("leaf", None),
        passthrough::Error::Transport(_) => error_row("transport", None),
        passthrough::Error::Collector { .. } => error_row("collector", None),
    }
}

struct SearchFixture {
    query: SearchInput,
    optional: IndexMap<String, Json>,
    bearer: Option<SecretString>,
    lookup: BTreeMap<String, String>,
}

impl SearchFixture {
    fn new(run: &SearchRun) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            query: SearchInput::Single(run.query.clone()),
            optional: serde_json::from_value(run.optional.clone().unwrap_or_else(|| json!({})))?,
            bearer: run.api_key.as_deref().map(token),
            lookup: BTreeMap::new(),
        })
    }

    fn operation<'call>(&'call self, run: &'call SearchRun) -> search::Operation<'call> {
        search::Operation {
            query: &self.query,
            optional_params: &self.optional,
            configured_tool_name: run.tool_name.as_deref(),
            headers: Headers::new(),
            configured_protocol_version: None,
            api_base: Some(GATEWAY),
            configured_gateway_url: None,
            session_region: None,
            api_key: self.bearer.as_ref(),
            credentials: credentials(),
            lookup: &self.lookup,
            now: fixture_time(),
        }
    }
}

pub(super) fn search_row(run: &SearchRun) -> super::HelperResult {
    let fixture = SearchFixture::new(run)?;
    let transport = Scripted {
        reply: reply(run.status, &run.reply),
    };
    match block_on(fixture.operation(run).run(&transport))? {
        Ok(outcome) => Ok(search_success(&outcome)),
        Err(search::Error::Leaf(_)) => Ok(error_row("leaf", None)),
        Err(search::Error::Reply(error)) => Ok(error_row("reply", error.status())),
        Err(search::Error::Transport(_)) => Ok(error_row("transport", None)),
    }
}

fn search_success(outcome: &search::Outcome) -> Value {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row(
            "POST",
            &outcome.request.url,
            &outcome.request.headers,
            &outcome.request.body,
        ),
    );
    row.insert("response".to_owned(), json!(outcome.response));
    Value::Object(row)
}

struct VectorFixture {
    query: SearchQuery,
    optional: SearchOptionalParams,
    litellm_params: IndexMap<String, Json>,
    extra_body: Option<IndexMap<String, Json>>,
}

impl VectorFixture {
    fn new(run: &VectorRun) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            query: SearchQuery::Text(run.query.clone()),
            optional: serde_json::from_value(run.optional.clone().unwrap_or_else(|| json!({})))?,
            litellm_params: serde_json::from_value(json!({"aws_region_name": "us-east-1"}))?,
            extra_body: run
                .extra_body
                .clone()
                .map(serde_json::from_value::<IndexMap<String, Json>>)
                .transpose()?,
        })
    }

    fn operation(&self) -> vector_retrieve::Operation<'_> {
        vector_retrieve::Operation {
            vector_store_id: VECTOR_ID,
            query: &self.query,
            optional: &self.optional,
            litellm_params: &self.litellm_params,
            extra_body: self.extra_body.as_ref(),
            api_base: None,
            configured_endpoint: None,
            environment_endpoint: None,
            aws_region_name: Some("us-east-1"),
            environment_region_name: None,
            environment_region: None,
            headers: BTreeMap::new(),
            signing_params: BTreeMap::new(),
            api_key: None,
            configured_bearer: None,
            credentials: credentials(),
            region_inputs: RegionInputs::default(),
            now: fixture_time(),
        }
    }
}

pub(super) fn vector_row(run: &VectorRun) -> super::HelperResult {
    let fixture = VectorFixture::new(run)?;
    let transport = Scripted {
        reply: reply(run.status, &run.reply),
    };
    match block_on(fixture.operation().run(&transport))? {
        Ok(outcome) => Ok(vector_success(&outcome)),
        Err(vector_retrieve::Error::Leaf(_)) => Ok(error_row("leaf", None)),
        Err(vector_retrieve::Error::Transport(_)) => Ok(error_row("transport", None)),
    }
}

fn vector_success(outcome: &vector_retrieve::Outcome) -> Value {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row("POST", &outcome.request.url, &[], &outcome.request.body),
    );
    row.insert("response".to_owned(), json!(outcome.response));
    Value::Object(row)
}

#[derive(Debug)]
struct Failing;

impl std::fmt::Display for Failing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("open failed")
    }
}

impl std::error::Error for Failing {}

impl ProviderClass for Failing {
    fn provider_class_name(&self) -> Option<&str> {
        Some("WebSocketError")
    }

    fn provider_message(&self) -> Option<&str> {
        Some("open failed")
    }
}

struct FailingRuntime;

impl RealtimeRuntime for FailingRuntime {
    type Error = Failing;

    fn connect(
        &self,
        _config: RuntimeConfig,
    ) -> impl Future<Output = Result<Box<dyn RealtimeClient<Error = Failing>>, StreamError<Failing>>>
    + Send {
        let outcome: Result<Box<dyn RealtimeClient<Error = Failing>>, StreamError<Failing>> =
            Err(StreamError {
                message: "the websocket could not open".to_owned(),
                source: Some(Failing),
            });
        ready(outcome)
    }
}

struct Resolver;

impl RealtimeCredentials for Resolver {
    fn resolve(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> impl Future<Output = Result<Option<Credentials>, CredentialError>> + Send {
        ready(Ok(Some(credentials())))
    }
}

struct Socket;

#[async_trait]
impl RealtimeClientWebSocket for Socket {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        Err(ClientError {
            message: "the client left".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, _data: String) -> Result<(), ClientError<io::Error>> {
        Ok(())
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

struct Silent;

impl RealtimeLogging for Silent {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, _key: &str, _value: Json) {}

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

struct NullIds;

impl IdSource for NullIds {
    fn next_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
}

fn transport_kind(error: &RealtimeTransportError<Failing, io::Error>) -> &'static str {
    match error {
        RealtimeTransportError::Committed(_) => "committed",
        RealtimeTransportError::Endpoint(_) => "endpoint",
        RealtimeTransportError::Credential(_) => "credential",
        RealtimeTransportError::NoCredentials { .. } => "no_credentials",
        RealtimeTransportError::Client(_) => "client",
        RealtimeTransportError::SessionCreated { .. } => "provider_unchanged",
        RealtimeTransportError::Stream(_) => "stream",
        RealtimeTransportError::Completion(_) => "completion",
        RealtimeTransportError::Logging { .. } => "logging",
        RealtimeTransportError::Provider(_) => "provider_bedrock",
    }
}

fn realtime_scope(run: &RealtimeRun) -> EventScope {
    EventScope {
        pending_session_update: Json::Text(run.pending.clone()),
        ..EventScope::default()
    }
}

pub(super) fn realtime_row(run: &RealtimeRun) -> super::HelperResult {
    let runtime = FailingRuntime;
    let resolver = Resolver;
    let socket = Socket;
    let logging = Silent;
    let ids = NullIds;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let logged_types = LoggedEventTypes::Default;
    let mut scope = realtime_scope(run);
    let call = RealtimeCall {
        model: REALTIME_MODEL,
        client: &socket,
        logging: &logging,
        logging_worker: None,
        api_base: None,
        aws_bedrock_runtime_endpoint: None,
        aws_region_name: Some("us-east-1"),
        auth: AuthParams::default(),
        region_inputs: RegionInputs::default(),
        scope: &mut scope,
        ids: &ids,
        redactor: &redactor,
        logged_types: &logged_types,
        trace_id: "trace-interactive",
    };
    let result = block_on(realtime_bridge::session(&runtime, &resolver, call))?;
    let mut row = empty_row();
    row.insert("response".to_owned(), realtime_response(result, &scope)?);
    Ok(Value::Object(row))
}

fn realtime_response(
    result: Result<realtime_bridge::Outcome, realtime_bridge::Error>,
    scope: &EventScope,
) -> Result<Value, Box<HelperError>> {
    let committed = matches!(&scope.session_committed, Json::Bool(true));
    match result {
        Ok(_) => Ok(json!({
            "outcome": "ok",
            "error_kind": Value::Null,
            "status_code": Value::Null,
            "committed": committed,
        })),
        Err(realtime_bridge::Error::Session { source }) => {
            let transport = source
                .downcast_ref::<RealtimeTransportError<Failing, io::Error>>()
                .ok_or_else(|| {
                    Box::new(HelperError {
                        class: "ValueError".to_owned(),
                        status: None,
                        error_type: Some(None),
                        param: Some(None),
                        code: Some(None),
                        message: "the realtime session error is not a transport error".to_owned(),
                    })
                })?;
            Ok(json!({
                "outcome": "error",
                "error_kind": transport_kind(transport),
                "status_code": transport.status_code(),
                "committed": committed,
            }))
        }
    }
}
