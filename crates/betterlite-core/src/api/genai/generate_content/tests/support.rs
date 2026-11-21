use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream::{self, once};
use http::HeaderMap as HttpHeaders;
use indexmap::IndexMap;

use crate::api::genai::chat_call::{
    CompletionCall, CompletionExecutor, CompletionOutput, Operation,
};
use crate::api::genai::generate_content::async_stream::agenerate_content_stream;
use crate::api::genai::generate_content::async_unary::agenerate_content;
use crate::api::genai::generate_content::error::GenerateContentFailure;
use crate::api::genai::generate_content::stream::generate_content_stream;
use crate::api::genai::generate_content::unary::generate_content;
use crate::api::genai::generate_content::{
    GenerateContentAnswer, GenerateContentDeps, GenerateContentRequest,
};
use crate::api::genai_http::config::{AuthCall, AuthTarget, GenerateContentHttpConfig};
use crate::api::genai_http::service::{
    ClientSource, GenAiRequest, GenAiTransport, StreamReply, WireReply,
};
use crate::api::genai_http::stream::StreamSettings;
use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, HeaderMap, header_map,
};
use crate::api::vector_store_http_write::service::{ClientKind, LogContext, SuppliedClient};
use crate::genai_setup::config::{
    GenerateContentProviderConfig, GenerateContentProviderConfigs,
    GenerateContentRequest as SetupRequest,
};
use crate::schema::SchemaLimits;

// gemini/ and openai/ models resolve to a known provider, which selects the native and the adapter path
pub(super) const NATIVE_MODEL: &str = "gemini/gemini-2.5-flash";
pub(super) const ADAPTER_MODEL: &str = "openai/gpt-4o";
pub(super) const CONTENTS: &str = r#"[{"role":"user","parts":[{"text":"Hi"}]}]"#;
pub(super) const NATIVE_REPLY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP"}]}"#;
pub(super) const CHAT_REPLY: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":0,"model":"gpt-4o","choices":[{"index":0,"message":{"role":"assistant","content":"hello there"},"finish_reason":"stop"}]}"#;
pub(super) const CHUNK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":0,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hello there"},"finish_reason":null}]}"#;
pub(super) const SSE: &str = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}],\"role\":\"model\"}}]}\n\n";
pub(super) const MOCK_TEXT: &str = "mocked answer";
pub(super) const DEFAULT_MOCK_TEXT: &str =
    "This is a mock response from Google GenAI generate_content.";
// the whole mock object LiteLLM answers, with TEXT standing in for the mock text in both places it appears
pub(super) const MOCK_TEMPLATE: &str = r#"{"text":"TEXT","candidates":[{"content":{"parts":[{"text":"TEXT"}],"role":"model"},"finishReason":"STOP","index":0,"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":20,"totalTokenCount":30}}"#;

pub(super) type Failure = GenerateContentFailure<TestConfigError, ScriptedFailure>;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture is valid")
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(super) fn contents() -> Json {
    json(CONTENTS)
}

// the record a test hands an entry point: the native fields start absent and the test sets the ones it needs
pub(super) fn request<'request>(
    model: &'request str,
    contents: &'request Json,
    provider: Option<&'request str>,
    kwargs: &'request Object,
) -> GenerateContentRequest<'request> {
    GenerateContentRequest {
        model,
        contents,
        config: None,
        tools: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: provider,
        kwargs,
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// the logging state every entry point fills, built the way the proxy builds it for one call
fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"gemini-2.5-flash","call_type":"generate_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the test provider config cannot build the call")]
pub(super) struct TestConfigError;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct ScriptedFailure(&'static str);

// what the wire saw: enough for a test to assert the URL, the headers, the body and the timeout the handler sent
#[derive(Clone)]
pub(super) struct Seen {
    pub(super) url: String,
    pub(super) headers: HttpHeaders,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Option<Duration>,
    pub(super) streaming: bool,
}

// the transport the handler sends through: it records the request and answers the fixture reply
#[derive(Clone)]
pub(super) struct Wire {
    status: u16,
    body: Vec<u8>,
    stream_body: Option<Vec<u8>>,
    requests: Arc<Mutex<Vec<Seen>>>,
}

impl Wire {
    pub(super) fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            stream_body: None,
            requests: Arc::default(),
        }
    }

    pub(super) fn streaming(status: u16, body: &str) -> Self {
        Self {
            stream_body: Some(body.as_bytes().to_vec()),
            ..Self::answering(status, body)
        }
    }
}

impl GenAiTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: GenAiRequest,
        streaming: bool,
    ) -> impl Future<Output = Result<WireReply, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Seen {
                url: request.url.to_string(),
                headers: request.headers,
                body: request.body,
                timeout: request.timeout,
                streaming,
            });
        let reply = match &self.stream_body {
            Some(body) if streaming => {
                let events = once(ready(Ok::<Vec<u8>, HttpError>(body.clone()))).boxed();
                WireReply::Streaming(StreamReply {
                    headers: HttpHeaders::new(),
                    body: events,
                })
            }
            Some(_) | None => WireReply::Buffered(UpstreamResponse {
                status: self.status,
                headers: HttpHeaders::new(),
                body: self.body.clone(),
            }),
        };
        ready(Ok(reply))
    }
}

pub(super) struct Source {
    wire: Wire,
}

impl Source {
    pub(super) const fn new(wire: Wire) -> Self {
        Self { wire }
    }

    pub(super) fn requests(&self) -> Vec<Seen> {
        self.wire
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, _ssl_verify: Option<&Json>) -> Result<Wire, ClientError> {
        Ok(self.wire.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuthRecord {
    pub(super) api_base: Option<String>,
    pub(super) model: String,
    pub(super) stream: bool,
}

// the provider config both native slots answer with: it records auth, builds a body and classes a failure
pub(super) struct TestConfig {
    base: String,
    pub(super) top_fields: &'static [&'static str],
    auth_calls: Mutex<Vec<AuthRecord>>,
    errors: Mutex<Vec<(String, u16)>>,
}

impl TestConfig {
    pub(super) fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            top_fields: &[],
            auth_calls: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    pub(super) fn auth_calls(&self) -> Vec<AuthRecord> {
        self.auth_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn errors(&self) -> Vec<(String, u16)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GenerateContentProviderConfig for TestConfig {
    type Error = TestConfigError;

    fn map_optional_params(&self, config: &Object, _model: &str) -> Object {
        config.clone()
    }

    fn transform_request(&self, request: &SetupRequest<'_>) -> Result<Json, TestConfigError> {
        let mut body = Object::new();
        body.insert("model".to_owned(), Json::Text(request.model.to_owned()));
        body.insert("contents".to_owned(), request.contents.clone());
        body.insert(
            "generationConfig".to_owned(),
            Json::Object(request.generation_config.clone()),
        );
        if let Some(tools) = request.tools {
            body.insert("tools".to_owned(), tools.clone());
        }
        if let Some(instruction) = request.system_instruction {
            body.insert("systemInstruction".to_owned(), instruction.clone());
        }
        Ok(Json::Object(body))
    }

    fn request_top_level_fields(&self) -> &'static [&'static str] {
        self.top_fields
    }
}

impl GenerateContentHttpConfig for TestConfig {
    type Response = Json;

    fn auth_token_and_url(
        &self,
        call: &AuthCall<'_>,
    ) -> impl Future<Output = Result<AuthTarget, TestConfigError>> + Send {
        self.auth_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(AuthRecord {
                api_base: call.api_base.map(str::to_owned),
                model: call.model.to_owned(),
                stream: call.stream,
            });
        let operation = if call.stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        let mut headers = IndexMap::new();
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers.insert("x-goog-api-key".to_owned(), "provider-key".to_owned());
        let target = AuthTarget {
            url: format!("{}/v1beta/models/{}:{operation}", self.base, call.model),
            headers,
        };
        ready(Ok(target))
    }

    fn decode_response(&self, response: &UpstreamResponse) -> Result<Json, BaseLlmException> {
        serde_json::from_slice(&response.body).map_err(|source| {
            self.error_class(
                &format!("Error transforming generate content response: {source}"),
                response.status,
                header_map(&response.headers),
            )
        })
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code));
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

// the completion pipeline the adapter calls into: it records the operation and answers the script
pub(super) struct Scripted {
    answer: Mutex<Option<Result<CompletionOutput, ScriptedFailure>>>,
    seen: Mutex<Vec<(Operation, CompletionCall)>>,
}

impl Scripted {
    pub(super) fn answering(output: CompletionOutput) -> Self {
        Self {
            answer: Mutex::new(Some(Ok(output))),
            seen: Mutex::default(),
        }
    }

    pub(super) fn failing(reason: &'static str) -> Self {
        Self {
            answer: Mutex::new(Some(Err(ScriptedFailure(reason)))),
            seen: Mutex::default(),
        }
    }

    pub(super) fn seen(&self) -> Vec<(Operation, CompletionCall)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        operation: Operation,
        call: CompletionCall,
    ) -> Result<CompletionOutput, ScriptedFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((operation, call));
        self.answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or(Err(ScriptedFailure("the scripted answer was used twice")))
    }
}

impl CompletionExecutor for Scripted {
    type Error = ScriptedFailure;

    fn completion(&self, call: CompletionCall) -> Result<CompletionOutput, ScriptedFailure> {
        self.record(Operation::Completion, call)
    }

    fn acompletion(
        &self,
        call: CompletionCall,
    ) -> impl Future<Output = Result<CompletionOutput, ScriptedFailure>> + Send {
        let answer = self.record(Operation::Acompletion, call);
        async move { answer }
    }
}

pub(super) fn response_output() -> CompletionOutput {
    let response: ChatResponse = serde_json::from_str(CHAT_REPLY).expect("the chat reply parses");
    CompletionOutput::Response(Box::new(response))
}

pub(super) fn stream_output() -> CompletionOutput {
    let chunk: Chunk = serde_json::from_str(CHUNK).expect("the chunk parses");
    CompletionOutput::Stream(stream::iter(vec![chunk]).boxed())
}

#[derive(Clone, Copy)]
pub(super) enum Entry {
    Generate,
    Agenerate,
    GenerateStream,
    AgenerateStream,
}

pub(super) struct Dispatch {
    pub(super) answer: Result<GenerateContentAnswer<Json>, Failure>,
    pub(super) state: LoggingState,
}

// every collaborator the four entry points take, wired over one test case and one recorded call
pub(super) struct Harness {
    pub(super) config: TestConfig,
    pub(super) source: Source,
    pub(super) supplied: Option<Source>,
    pub(super) executor: Scripted,
    pub(super) redactor: Redactor,
    pub(super) capabilities: CatalogCapabilities,
    pub(super) catalog: ModelCatalog,
    pub(super) names: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) timeout: RequestTimeout,
    pub(super) now: DateTime<FixedOffset>,
}

impl Harness {
    pub(super) fn new() -> Self {
        Self {
            config: TestConfig::new("https://gemini.test"),
            source: Source::new(Wire::answering(200, NATIVE_REPLY)),
            supplied: None,
            executor: Scripted::answering(response_output()),
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            capabilities: CatalogCapabilities {
                chat_models: BTreeSet::from([String::from("gpt-4o")]),
                text_models: BTreeSet::new(),
            },
            catalog: ModelCatalog::default(),
            names: BTreeSet::from([String::from("gemini"), String::from("openai")]),
            environment: BTreeMap::new(),
            timeout: RequestTimeout {
                seconds: 6000.0_f64,
                explicitly_set: false,
            },
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    pub(super) async fn call(
        &self,
        entry: Entry,
        request: &GenerateContentRequest<'_>,
    ) -> Dispatch {
        let configs = GenerateContentProviderConfigs {
            gemini: &self.config,
            vertex: &self.config,
            is_vertex_partner_model: |_model| false,
        };
        let resolution = ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let supplied_client = self.supplied.as_ref().map(|source| SuppliedClient {
            kind: ClientKind::Sync,
            transport: &source.wire,
        });
        let deps = GenerateContentDeps {
            configs: &configs,
            resolution: &resolution,
            capabilities: &self.capabilities,
            limits: SchemaLimits::default(),
            source: &self.source,
            supplied_client,
            executor: &self.executor,
            stream: StreamSettings::default(),
            request_timeout: self.timeout,
            redactor: &self.redactor,
        };
        let sink = Sink;
        let env = pre_post_env(&self.redactor, &sink, self.now);
        let mut state = logging_state(&self.redactor);
        let answer = {
            let mut log = LogContext {
                state: &mut state,
                env: &env,
            };
            run_entry(&deps, entry, request, &mut log).await
        };
        Dispatch { answer, state }
    }
}

// the entry point the case runs, over the deps wired above
async fn run_entry<C, S, Executor>(
    deps: &GenerateContentDeps<'_, C, S, Executor>,
    entry: Entry,
    request: &GenerateContentRequest<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<GenerateContentAnswer<C::Response>, GenerateContentFailure<C::Error, Executor::Error>>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor + Sync,
{
    match entry {
        Entry::Generate => generate_content(deps, request, log).await,
        Entry::Agenerate => agenerate_content(deps, request, log).await,
        Entry::GenerateStream => generate_content_stream(deps, request, log).await,
        Entry::AgenerateStream => agenerate_content_stream(deps, request, log).await,
    }
}
