#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::passthrough::error::PassthroughFailure;
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_bedrock::search::error::SearchError;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::error::VectorStoreError;
use betterlite_bedrock::vector_stores::request::{SearchOptionalParams, SearchQuery};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::bedrock_services_search::DEFAULT_SEARCH_TOOL_NAME;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_openai::search::provider::SearchQuery as SearchInput;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::error::BedrockInteractiveError;
use super::http::{LeafReply, LeafRequest, LeafTransport};
use super::{passthrough, responses, search, vector_retrieve};
use dispatch_support::dispatched;

const HOST: &str = "https://bedrock-runtime.us-east-1.amazonaws.com";
const RESPONSES_HOST: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";
const RESPONSES_MODEL: &str = "openai.gpt-oss-120b";
const GATEWAY: &str = "https://gateway.example.com/mcp";
const ACCESS: &str = "AKIDBETTERLITEFIXTURE";
const SECRET: &str = "fixture/secret/key/+EXAMPLEKEY0000";
const SESSION_UPDATE: &str = r#"{"type":"session.update","session":{}}"#;
const CONVERSE_FRAMES: &str = "0000009300000057068b46fe0b3a6576656e742d74797065070011636f6e74656e74426c6f636b44656c74610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b22636f6e74656e74426c6f636b496e646578223a302c2264656c7461223a7b2274657874223a2261227d7d13198e870000009300000057068b46fe0b3a6576656e742d74797065070011636f6e74656e74426c6f636b44656c74610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b22636f6e74656e74426c6f636b496e646578223a302c2264656c7461223a7b2274657874223a2262227d7d01ac21690000007a00000051ca2c46650b3a6576656e742d7479706507000b6d65737361676553746f700d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b2273746f70526561736f6e223a22656e645f7475726e227d5fbf09fc000000b40000004e11011d2a0b3a6576656e742d747970650700086d657461646174610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b226d657472696373223a7b226c6174656e63794d73223a317d2c227573616765223a7b22696e707574546f6b656e73223a332c226f7574707574546f6b656e73223a342c22746f74616c546f6b656e73223a377d7d6ac04a01";

fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_hours(490_896))
        .unwrap()
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

fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn pjson(source: &str) -> PromptJson {
    serde_json::from_str(source).unwrap()
}

fn text_field<'body>(object: &'body Object, name: &str) -> Option<&'body str> {
    object.get(name).and_then(PromptJson::as_str)
}

fn object_field<'body>(object: &'body Object, name: &str) -> Option<&'body Object> {
    object.get(name).and_then(PromptJson::as_object)
}

fn from_serialized<T: serde::Serialize>(value: &T) -> Json {
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

fn body_object(body: &[u8]) -> Object {
    serde_json::from_slice(body).unwrap()
}

fn header<'text>(headers: &'text [(String, Option<String>)], name: &str) -> Option<&'text str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_deref())
}

struct RecordingTransport {
    reply: LeafReply,
    requests: Mutex<Vec<LeafRequest>>,
}

impl RecordingTransport {
    fn new(reply: LeafReply) -> Self {
        Self {
            reply,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<LeafRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn only_request(&self) -> LeafRequest {
        let requests = self.requests();
        assert_eq!(requests.len(), 1);
        requests.into_iter().next().unwrap()
    }
}

impl LeafTransport for RecordingTransport {
    fn send(
        &self,
        request: LeafRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LeafReply, TransportError>> + Send + '_>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Box::pin(ready(Ok(self.reply.clone())))
    }
}

struct Capabilities;

impl ReasoningCapabilities for Capabilities {
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

impl ResponsesCapabilities for Capabilities {
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

struct Invocation<'call> {
    model: Option<&'call str>,
    api_key: Option<&'call SecretString>,
}

fn responses_operation<'call>(
    input: &'call ResponsesInput,
    params: &'call ResponsesParams,
    model_cost: &'call ParamMap,
    capabilities: &'call mut Capabilities,
    invocation: Invocation<'call>,
) -> responses::Operation<'call, MapFetcher, Capabilities> {
    responses::Operation {
        aws: responses::AwsRequest {
            credentials: credentials(),
            api_base: Some(RESPONSES_HOST),
            aws_region_name: Some("us-west-2"),
            aws_bedrock_runtime_endpoint: None,
            environment_endpoint: None,
            region_inputs: RegionInputs::default(),
            api_key: invocation.api_key,
            environment_token: None,
            headers: IndexMap::new(),
            signing_params: BTreeMap::new(),
            now: fixture_time(),
        },
        body: responses::BodyRequest {
            input,
            params,
            model_info: None,
        },
        model: invocation.model,
        model_cost,
        fetcher: MapFetcher,
        capabilities,
        drop_params: DropParams {
            global: false,
            request: false,
        },
        bundled: BundledReasoningModels::default(),
    }
}

const RESPONSES_INPUT: &str = r#"[{"type":"message","role":"user","content":"keep"}]"#;
const RESPONSES_PARAMS: &str = r#"{"background": true, "temperature": 0.5, "tools": [{"type": "function", "name": "f"}, {"type": "web_search"}]}"#;

fn responses_cost() -> ParamMap {
    serde_json::from_str(r#"{"openai.gpt-oss-120b": {"supported_endpoints": ["/v1/responses"]}}"#)
        .unwrap()
}

fn responses_body() -> Object {
    let input: ResponsesInput = serde_json::from_str(RESPONSES_INPUT).unwrap();
    let params: ResponsesParams = serde_json::from_str(RESPONSES_PARAMS).unwrap();
    let model_cost = responses_cost();
    let mut capabilities = Capabilities;
    let bearer = token("fixture-token");
    let operation = responses_operation(
        &input,
        &params,
        &model_cost,
        &mut capabilities,
        Invocation {
            model: Some(RESPONSES_MODEL),
            api_key: Some(&bearer),
        },
    );
    let reply = LeafReply {
        status: 200,
        headers: Vec::new(),
        body: br#"{"id":"resp_1"}"#.to_vec(),
        chunks: Vec::new(),
    };
    let transport = RecordingTransport::new(reply);
    let outcome = dispatched!(Responses, operation.run(&transport)).unwrap();
    let request = transport.only_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, format!("{RESPONSES_HOST}/openai/v1/responses"));
    assert_eq!(
        header(&request.headers, "Authorization"),
        Some("Bearer fixture-token")
    );
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.body, br#"{"id":"resp_1"}"#.to_vec());
    body_object(&request.body)
}

fn responses_case() {
    let body = responses_body();
    assert_eq!(text_field(&body, "model"), Some(RESPONSES_MODEL));
    assert_eq!(body.get("input"), Some(&pjson(RESPONSES_INPUT)));
    assert_eq!(body.get("background"), None);
    assert_eq!(body.get("temperature"), Some(&pjson("0.5")));
    assert_eq!(
        body.get("tools"),
        Some(&pjson(r#"[{"type":"function","name":"f"}]"#))
    );
    responses_remote_image();
    responses_unsupported_model();
    responses_rejected_reply();
}

fn responses_rejected_reply() {
    let input: ResponsesInput = serde_json::from_str(r#""plain text""#).unwrap();
    let params: ResponsesParams = serde_json::from_str("{}").unwrap();
    let model_cost = responses_cost();
    let mut capabilities = Capabilities;
    let operation = responses_operation(
        &input,
        &params,
        &model_cost,
        &mut capabilities,
        Invocation {
            model: Some(RESPONSES_MODEL),
            api_key: None,
        },
    );
    let transport = RecordingTransport::new(LeafReply {
        status: 400,
        headers: Vec::new(),
        body: b"bad \xff body".to_vec(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Responses, operation.run(&transport)).unwrap_err();
    match error {
        responses::Error::Rejected {
            status, message, ..
        } => {
            assert_eq!(status, 400);
            assert_eq!(message, "bad \u{FFFD} body");
        }
        other => panic!("expected a rejected reply, got {other}"),
    }
}

fn responses_remote_image() {
    let input: ResponsesInput = serde_json::from_str(
        r#"[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://images.example/cat.png"}]}]"#,
    )
    .unwrap();
    let params: ResponsesParams = serde_json::from_str("{}").unwrap();
    let model_cost = responses_cost();
    let mut capabilities = Capabilities;
    let operation = responses_operation(
        &input,
        &params,
        &model_cost,
        &mut capabilities,
        Invocation {
            model: Some(RESPONSES_MODEL),
            api_key: None,
        },
    );
    let reply = LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: Vec::new(),
    };
    let transport = RecordingTransport::new(reply);
    let error = dispatched!(Responses, operation.run(&transport)).unwrap_err();
    assert!(matches!(error, responses::Error::Leaf(_)));
    assert!(transport.requests().is_empty());
}

fn responses_unsupported_model() {
    let input: ResponsesInput = serde_json::from_str(r#""plain text""#).unwrap();
    let params: ResponsesParams = serde_json::from_str("{}").unwrap();
    let model_cost = ParamMap::new();
    let mut capabilities = Capabilities;
    let operation = responses_operation(
        &input,
        &params,
        &model_cost,
        &mut capabilities,
        Invocation {
            model: Some("mystery.model"),
            api_key: None,
        },
    );
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Responses, operation.run(&transport)).unwrap_err();
    match error {
        responses::Error::UnsupportedModel { model } => assert_eq!(model, "mystery.model"),
        other => panic!("expected an unsupported model, got {other}"),
    }
}

struct TestLogging {
    calls: Mutex<Vec<RelayCallType>>,
}

impl TestLogging {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl PassthroughLogging for TestLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call_type);
    }
}

struct TestSelector {
    tag: Option<ChatConfigTag>,
}

impl ChatConfigSelection for TestSelector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(self.tag)
    }
}

struct TestTransform {
    result: Json,
}

impl betterlite_bedrock::passthrough::reply::NativeReplyTransform for TestTransform {
    fn transform(
        &self,
        _request: &betterlite_bedrock::passthrough::reply::NativeReplyRequest<'_>,
    ) -> Result<Json, Box<ProviderError>> {
        Ok(self.result.clone())
    }
}

struct TestAssembler {
    chunks: Mutex<Vec<StreamChunk>>,
}

impl TestAssembler {
    fn new() -> Self {
        Self {
            chunks: Mutex::new(Vec::new()),
        }
    }

    fn count(&self) -> usize {
        self.chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl betterlite_bedrock::passthrough::collector::StreamLogAssembler for TestAssembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        *self.chunks.lock().unwrap_or_else(PoisonError::into_inner) = chunks.to_vec();
        if chunks.is_empty() {
            return None;
        }
        Some(json(r#"{"id":"assembled"}"#))
    }
}

fn fixture_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_767_225_600,
        timestamp: 1_767_225_600.0,
    }
}

fn passthrough_plan<'services>(
    selector: &'services TestSelector,
    transform: &'services TestTransform,
    assembler: &'services TestAssembler,
    credentials: Option<Credentials>,
) -> BedrockPassthroughPlan<'services> {
    BedrockPassthroughPlan {
        environment: PassthroughEnvironment {
            region_name: Some("us-east-1".to_owned()),
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
        identity: PassthroughIdentity {
            credentials,
            bearer_token: None,
            now: fixture_time(),
        },
        selector,
        transform,
        assembler,
        clock: CollectorClock {
            next_id: || uuid::Uuid::from_u128(7),
            now: || 1_767_225_600,
            instant: fixture_instant,
        },
    }
}

fn invoke_target() -> PassthroughTarget<'static> {
    PassthroughTarget {
        model: "anthropic.claude-3",
        custom_llm_provider: "bedrock",
        endpoint: "model/anthropic.claude-3/invoke",
    }
}

fn invoke_url() -> String {
    format!("{HOST}/model/anthropic.claude-3/invoke")
}

fn passthrough_operation<'call>(
    plan: BedrockPassthroughPlan<'call>,
    url_request: PassthroughUrlRequest<'call>,
    signing: PassthroughSigning<'call>,
    logging: &'call mut TestLogging,
) -> passthrough::Operation<'call> {
    passthrough::Operation {
        plan,
        url_request,
        signing,
        headers: Headers::new(),
        logging,
    }
}

fn passthrough_buffered() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform {
        result: json(r#"{"id":"logged"}"#),
    };
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, Some(credentials()));
    let params = object("{}");
    let data = object(r#"{"input":"hi"}"#);
    let signed_base = invoke_url();
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: invoke_target(),
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging::new();
    let operation = passthrough_operation(plan, url_request, signing, &mut logging);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: br#"{"id":"native"}"#.to_vec(),
        chunks: Vec::new(),
    });
    let outcome = dispatched!(Passthrough, operation.run(&transport)).unwrap();
    assert_buffered(&outcome);
}

fn assert_buffered(outcome: &passthrough::Outcome) {
    assert_eq!(outcome.endpoint_url, HOST);
    assert_eq!(outcome.request.url, invoke_url());
    assert!(!outcome.streaming);
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.logged, Some(json(r#"{"id":"logged"}"#)));
    assert_eq!(outcome.request.body, br#"{"input": "hi"}"#.to_vec());
    let authorization = header(&outcome.request.headers, "Authorization").unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDBETTERLITEFIXTURE/"));
    assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
}

fn passthrough_streaming() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform {
        result: json("null"),
    };
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, Some(credentials()));
    let params = object("{}");
    let data = object(r#"{"stream":true}"#);
    let endpoint = "model/anthropic.claude-3/converse-stream";
    let signed_base = format!("{HOST}/{endpoint}");
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: PassthroughTarget {
            model: "anthropic.claude-3",
            custom_llm_provider: "bedrock",
            endpoint,
        },
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging::new();
    let operation = passthrough_operation(plan, url_request, signing, &mut logging);
    let transport = converse_transport();
    let outcome = dispatched!(Passthrough, operation.run(&transport)).unwrap();
    assert_streaming(&outcome, &assembler);
}

fn converse_transport() -> RecordingTransport {
    RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: vec![hex::decode(CONVERSE_FRAMES).unwrap()],
    })
}

fn assert_streaming(outcome: &passthrough::Outcome, assembler: &TestAssembler) {
    assert!(outcome.streaming);
    assert_eq!(outcome.logged, Some(json(r#"{"id":"assembled"}"#)));
    assert!(assembler.count() > 0);
}

fn passthrough_missing_credentials() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform {
        result: json("null"),
    };
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, None);
    let params = object("{}");
    let signed_base = invoke_url();
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: invoke_target(),
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: None,
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging::new();
    let operation = passthrough_operation(plan, url_request, signing, &mut logging);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Passthrough, operation.run(&transport)).unwrap_err();
    match error {
        passthrough::Error::Leaf(PassthroughFailure::Signing(_)) => {}
        other => panic!("expected a signing failure, got {other}"),
    }
    assert!(transport.requests().is_empty());
}

fn passthrough_case() {
    passthrough_buffered();
    passthrough_streaming();
    passthrough_missing_credentials();
}

const SEARCH_REPLY: &str = "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"[{\\\"title\\\":\\\"Docs\\\",\\\"url\\\":\\\"https://docs.example.com\\\",\\\"text\\\":\\\"snippet\\\",\\\"publishedDate\\\":\\\"2026-01-01\\\"}]\"}]}}";

fn search_operation<'call>(
    query: &'call SearchInput,
    optional: &'call IndexMap<String, Json>,
    configured: Option<&'call str>,
    api_key: Option<&'call SecretString>,
    lookup: &'call (dyn EnvironmentLookup + Sync),
) -> search::Operation<'call> {
    search::Operation {
        query,
        optional_params: optional,
        configured_tool_name: configured,
        headers: Headers::new(),
        configured_protocol_version: None,
        api_base: Some(GATEWAY),
        configured_gateway_url: None,
        session_region: None,
        api_key,
        credentials: credentials(),
        lookup,
        now: fixture_time(),
    }
}

fn assert_search_headers(headers: &[(String, Option<String>)]) {
    assert_eq!(
        header(headers, "Authorization"),
        Some("Bearer gateway-token")
    );
    assert_eq!(header(headers, "Content-Type"), Some("application/json"));
    assert_eq!(
        header(headers, "Accept"),
        Some("application/json, text/event-stream")
    );
    assert_eq!(header(headers, "MCP-Protocol-Version"), Some("2025-03-26"));
}

fn search_case() {
    let query = SearchInput::Single("bedrock agents".to_owned());
    let optional: IndexMap<String, Json> = serde_json::from_str(r#"{"max_results": 3}"#).unwrap();
    let bearer = token("gateway-token");
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let operation = search_operation(&query, &optional, None, Some(&bearer), &lookup);
    let reply = LeafReply {
        status: 200,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: SEARCH_REPLY.as_bytes().to_vec(),
        chunks: Vec::new(),
    };
    let transport = RecordingTransport::new(reply);
    let outcome = dispatched!(Search, operation.run(&transport)).unwrap();
    let request = transport.only_request();
    assert_eq!(request.url, GATEWAY);
    assert_eq!(request.method, "POST");
    assert_search_headers(&request.headers);
    let body = body_object(&request.body);
    let params = object_field(&body, "params").unwrap();
    assert_eq!(text_field(params, "name"), Some(DEFAULT_SEARCH_TOOL_NAME));
    let arguments = object_field(params, "arguments").unwrap();
    assert_eq!(text_field(arguments, "query"), Some("bedrock agents"));
    assert_eq!(arguments.get("maxResults"), Some(&pjson("3")));
    assert_eq!(outcome.response.results.len(), 1);
    assert_eq!(outcome.response.results[0].title, "Docs");
    assert_eq!(outcome.response.results[0].snippet, "snippet");
    search_reply_error();
}

#[test]
fn search_leaf_failure() {
    let query = SearchInput::Single("q".to_owned());
    let optional = IndexMap::new();
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let operation = search_operation(&query, &optional, Some("other-tool"), None, &lookup);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: SEARCH_REPLY.as_bytes().to_vec(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Search, operation.run(&transport)).unwrap_err();
    match error {
        search::Error::Leaf(SearchError::ToolName { tool_name }) => {
            assert_eq!(tool_name, "other-tool");
        }
        other => panic!("expected a tool name failure, got {other}"),
    }
    assert!(transport.requests().is_empty());
}

fn search_reply_error() {
    let query = SearchInput::Single("q".to_owned());
    let optional = IndexMap::new();
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let operation = search_operation(&query, &optional, None, None, &lookup);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"bad"}}"#.to_vec(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Search, operation.run(&transport)).unwrap_err();
    match error {
        search::Error::Reply(reply) => assert_eq!(reply.status(), Some(502)),
        other => panic!("expected a reply error, got {other}"),
    }
}

const VECTOR_REPLY: &str = concat!(
    r#"{"retrievalResults":[{"content":{"text":"found"},"score":0.9,"#,
    r#""metadata":{"x-amz-bedrock-kb-source-uri":"s3://bucket/doc.pdf"}}]}"#
);

fn vector_operation<'call>(
    query: &'call SearchQuery,
    optional: &'call SearchOptionalParams,
    litellm_params: &'call IndexMap<String, Json>,
    extra_body: Option<&'call IndexMap<String, Json>>,
) -> vector_retrieve::Operation<'call> {
    vector_retrieve::Operation {
        vector_store_id: "KB123",
        query,
        optional,
        litellm_params,
        extra_body,
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

fn vector_case() {
    let query = SearchQuery::Text("hello".to_owned());
    let optional = SearchOptionalParams {
        filters: Some(json(r#"{"equals":{"key":"k","value":"v"}}"#)),
        max_num_results: Some(5),
        ranking_options: None,
        rewrite_query: None,
    };
    let litellm_params: IndexMap<String, Json> =
        serde_json::from_str(r#"{"aws_region_name":"us-east-1"}"#).unwrap();
    let extra: IndexMap<String, Json> =
        serde_json::from_str(r#"{"userContext":{"user":"u"}}"#).unwrap();
    let operation = vector_operation(&query, &optional, &litellm_params, Some(&extra));
    let reply = LeafReply {
        status: 200,
        headers: Vec::new(),
        body: VECTOR_REPLY.as_bytes().to_vec(),
        chunks: Vec::new(),
    };
    let transport = RecordingTransport::new(reply);
    let outcome = dispatched!(VectorRetrieve, operation.run(&transport)).unwrap();
    let request = transport.only_request();
    assert_vector_request(&request);
    assert_vector_response(&from_serialized(&outcome.response));
}

fn assert_vector_request(request: &LeafRequest) {
    assert_eq!(
        request.url,
        "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases/KB123/retrieve"
    );
    let authorization = header(&request.headers, "Authorization").unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDBETTERLITEFIXTURE/"));
    assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
    let body = body_object(&request.body);
    let retrieval = object_field(&body, "retrievalQuery").unwrap();
    assert_eq!(text_field(retrieval, "text"), Some("hello"));
    let configuration = object_field(&body, "retrievalConfiguration").unwrap();
    let search = object_field(configuration, "vectorSearchConfiguration").unwrap();
    assert_eq!(search.get("numberOfResults"), Some(&pjson("5")));
    assert_eq!(
        search.get("filter"),
        Some(&pjson(r#"{"equals":{"key":"k","value":"v"}}"#))
    );
    assert_eq!(body.get("userContext"), Some(&pjson(r#"{"user":"u"}"#)));
}

fn assert_vector_response(response: &Json) {
    let Json::Object(envelope) = response else {
        panic!("the vector response is an object");
    };
    let Some(Json::List(data)) = envelope.get("data") else {
        panic!("the vector response carries a data list");
    };
    let Some(Json::Object(first)) = data.first() else {
        panic!("the vector response carries one result");
    };
    assert_eq!(
        first.get("file_id"),
        Some(&Json::Text("s3://bucket/doc.pdf".to_owned()))
    );
    assert_eq!(
        first.get("filename"),
        Some(&Json::Text("doc.pdf".to_owned()))
    );
    assert_eq!(first.get("score"), Some(&json("0.9")));
}

#[test]
fn vector_leaf_failure() {
    let query = SearchQuery::Text("hello".to_owned());
    let optional = SearchOptionalParams {
        filters: None,
        max_num_results: Some(5),
        ranking_options: None,
        rewrite_query: None,
    };
    let litellm_params = IndexMap::new();
    let extra: IndexMap<String, Json> =
        serde_json::from_str(r#"{"retrievalConfiguration":"nope"}"#).unwrap();
    let operation = vector_operation(&query, &optional, &litellm_params, Some(&extra));
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: VECTOR_REPLY.as_bytes().to_vec(),
        chunks: Vec::new(),
    });
    let error = dispatched!(VectorRetrieve, operation.run(&transport)).unwrap_err();
    match error {
        vector_retrieve::Error::Leaf(VectorStoreError::RetrievalConfigNotObject) => {}
        other => panic!("expected a retrieval configuration failure, got {other}"),
    }
    assert!(transport.requests().is_empty());
}

mod dispatch_support;
mod realtime_support;

use realtime_support::realtime_case;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    Responses,
    Passthrough,
    Search,
    VectorRetrieve,
    RealtimeBridge,
}

#[rstest]
#[case::responses(Scenario::Responses)]
#[case::passthrough(Scenario::Passthrough)]
#[case::search(Scenario::Search)]
#[case::vector_retrieve(Scenario::VectorRetrieve)]
#[case::realtime_bridge(Scenario::RealtimeBridge)]
fn bedrock_interactive_dispatch_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::Responses => responses_case(),
        Scenario::Passthrough => passthrough_case(),
        Scenario::Search => search_case(),
        Scenario::VectorRetrieve => vector_case(),
        Scenario::RealtimeBridge => realtime_case(),
    }
}

#[test]
fn bedrock_interactive_dispatch_unsupported_operation() {
    let error = block_on(super::dispatch(
        super::BedrockInteractiveOperation::Unsupported {
            operation: "embedding",
        },
    ))
    .expect_err("an operation this task does not own is refused");
    assert_eq!(
        error.to_string(),
        "betterlite does not run the embedding operation through the Bedrock interactive provider"
    );
    match error {
        BedrockInteractiveError::Unsupported { operation } => assert_eq!(operation, "embedding"),
        other => panic!("expected an unsupported operation, got {other}"),
    }
}
