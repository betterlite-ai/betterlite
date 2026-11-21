#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::mem;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::speech_call::Settings;
use betterlite_context::media_config_selection::text_to_speech::TextToSpeechConfigTag;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::files::transport::StreamReply;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::result::SpeechTransport;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use crate::api::speech_call::call::SpeechCall;
use crate::api::speech_call::error::{SpeechCallError, SpeechException};
use crate::api::speech_call::logging::SpeechCallLogging;
use crate::api::speech_call::operations::{
    SpeechConfigOperation, SpeechConfigSource, SpeechMappingOptions, SpeechOperation,
    SpeechOperationKind, SpeechOperations,
};
use crate::api::speech_call::service::SpeechDispatch;
use crate::api::speech_chat_bridge::{
    SpeechCompletionAnswer, SpeechCompletionCall, SpeechCompletionOutput, SpeechCompletions,
};

pub(super) const SOURCE: &str = "litellm_settings: {}\n";
const INIT: &str = r#"{"model":"speech-test","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(super) struct Fixture {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    pub(super) fn with_environment(variables: &[(&str, &str)]) -> Self {
        let environment = map(variables);
        Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Reject)
                .expect("the empty catalog parses"),
            names: BTreeSet::new(),
            environment,
        }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
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

fn map(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn loaded_settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup = map(variables);
    let loaded = ConfigLoader { lookup }
        .load(source)
        .expect("the settings load");
    loaded.sections.speech_call
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(super) fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init"),
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || String::from("trace"),
        Duration::default,
    )
    .expect("the logging state")
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime")
}

pub(super) fn base_call(model: &str) -> SpeechCall {
    SpeechCall {
        model: model.to_owned(),
        input: String::from("hello"),
        voice: None,
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        project: None,
        custom_llm_provider: None,
        base_url: None,
        max_retries: None,
        timeout_seconds: None,
        metadata: None,
        response_format: None,
        speed: None,
        instructions: None,
        headers: None,
        client: None,
        shared_session: None,
        aspeech: false,
        kwargs: Object::new(),
    }
}

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a JSON object")
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a JSON value")
}

pub(super) fn named_voice(name: &str) -> SpeechVoice {
    SpeechVoice::Name(name.to_owned())
}

pub(super) fn dict_voice(source: &str) -> SpeechVoice {
    SpeechVoice::Options(object(source))
}

pub(super) struct RecordingTransport {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    sent: Mutex<Vec<OutboundRequest>>,
}

impl RecordingTransport {
    pub(super) fn ok(content_type: &str, body: &[u8]) -> Self {
        Self::building(
            200,
            vec![
                (String::from("content-type"), content_type.to_owned()),
                (String::from("x-request-id"), String::from("req-1")),
            ],
            body,
        )
    }

    pub(super) fn error(status: u16, body: &[u8]) -> Self {
        Self::building(
            status,
            vec![(
                String::from("content-type"),
                String::from("application/json"),
            )],
            body,
        )
    }

    fn building(status: u16, headers: Vec<(String, String)>, body: &[u8]) -> Self {
        Self {
            status,
            headers,
            body: body.to_vec(),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn sent(&self) -> Vec<OutboundRequest> {
        let mut sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *sent)
    }
}

impl SpeechTransport for RecordingTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).expect("the fixture header name"),
                HeaderValue::from_str(value).expect("the fixture header value"),
            );
        }
        ready(Ok(StreamReply {
            status: self.status,
            headers,
            body: stream::iter(vec![Ok(self.body.clone())]).boxed(),
        }))
    }
}

pub(super) struct BrokenTransport;

impl SpeechTransport for BrokenTransport {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

pub(super) fn sent_json(request: &OutboundRequest) -> Value {
    match &request.body {
        RequestBody::Json(bytes) => serde_json::from_slice(bytes).expect("the sent body is JSON"),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            panic!("the speech call sends a JSON body")
        }
    }
}

pub(super) fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(super) struct MappingCall {
    pub(super) model: String,
    pub(super) options: SpeechMappingOptions,
}

// the voice a scripted mapping applies: the caller's, or the one the mapping sets
pub(super) enum VoiceMapping {
    Keep,
    Set(Option<SpeechVoice>),
}

pub(super) struct ScriptedConfig {
    voice: VoiceMapping,
    appended: Object,
    pub(super) calls: Mutex<Vec<MappingCall>>,
}

impl ScriptedConfig {
    pub(super) fn keeping(appended: &str) -> Self {
        Self {
            voice: VoiceMapping::Keep,
            appended: object(appended),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn mapped(voice: Option<SpeechVoice>, appended: &str) -> Self {
        Self {
            voice: VoiceMapping::Set(voice),
            appended: object(appended),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<MappingCall> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *calls)
    }
}

impl SpeechConfigOperation for ScriptedConfig {
    fn map_openai_params(
        &self,
        model: &str,
        mut optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<SpeechVoice>, Object) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(MappingCall {
                model: model.to_owned(),
                options: options.clone(),
            });
        optional_params.extend(self.appended.clone());
        let voice = match &self.voice {
            VoiceMapping::Keep => options.voice.clone(),
            VoiceMapping::Set(voice) => voice.clone(),
        };
        (voice, optional_params)
    }
}

pub(super) struct Configs {
    lyria: bool,
    mapping: Option<ScriptedConfig>,
}

impl Configs {
    pub(super) fn absent() -> Self {
        Self {
            lyria: false,
            mapping: None,
        }
    }

    pub(super) fn scripted(mapping: ScriptedConfig) -> Self {
        Self {
            lyria: false,
            mapping: Some(mapping),
        }
    }

    pub(super) fn lyria(mapping: ScriptedConfig) -> Self {
        Self {
            lyria: true,
            mapping: Some(mapping),
        }
    }

    pub(super) fn mapping(&self) -> Option<&ScriptedConfig> {
        self.mapping.as_ref()
    }
}

impl SpeechConfigSource for Configs {
    fn is_lyria_model(&self, _model: &str) -> bool {
        self.lyria
    }

    fn config(&self, _tag: TextToSpeechConfigTag) -> Option<&dyn SpeechConfigOperation> {
        let mapping: &dyn SpeechConfigOperation = self.mapping.as_ref()?;
        Some(mapping)
    }
}

pub(super) struct OperationRecord {
    pub(super) kind: SpeechOperationKind,
    pub(super) provider: String,
    pub(super) config_tag: Option<TextToSpeechConfigTag>,
    pub(super) config_present: bool,
    pub(super) voice: Option<SpeechVoice>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) timeout_seconds: f64,
    pub(super) extra_headers: Option<Object>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) organization: Option<String>,
    pub(super) max_retries: u32,
    pub(super) azure_ad_token: Option<String>,
    pub(super) azure_ad_token_provider: Option<Value>,
    pub(super) kwargs: Object,
}

impl From<&SpeechOperation<'_>> for OperationRecord {
    fn from(operation: &SpeechOperation<'_>) -> Self {
        Self {
            kind: operation.kind,
            provider: operation.provider.clone(),
            config_tag: operation.config_tag,
            config_present: operation.config.is_some(),
            voice: operation.voice.clone(),
            optional_params: operation.optional_params.clone(),
            litellm_params: operation.litellm_params.clone(),
            timeout_seconds: operation.timeout_seconds,
            extra_headers: operation.extra_headers.clone(),
            api_base: operation.api_base.clone(),
            api_key: operation
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            api_version: operation.api_version.clone(),
            organization: operation.organization.clone(),
            max_retries: operation.max_retries,
            azure_ad_token: operation
                .azure_ad_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            azure_ad_token_provider: operation.azure_ad_token_provider.clone(),
            kwargs: operation.kwargs.clone(),
        }
    }
}

pub(super) struct ScriptedOperations {
    body: Vec<u8>,
    executed: Mutex<Vec<OperationRecord>>,
}

impl ScriptedOperations {
    pub(super) fn ok(body: &[u8]) -> Self {
        Self {
            body: body.to_vec(),
            executed: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn take(&self) -> Vec<OperationRecord> {
        let mut executed = self.executed.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *executed)
    }
}

impl SpeechOperations for ScriptedOperations {
    fn execute(
        &self,
        operation: SpeechOperation<'_>,
    ) -> impl Future<Output = Result<BinaryContent, SpeechCallError>> + Send {
        self.executed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(OperationRecord::from(&operation));
        ready(Ok(BinaryContent {
            status: 200,
            headers: IndexMap::from([(String::from("content-type"), String::from("audio/wav"))]),
            body: self.body.clone(),
        }))
    }
}

enum CompletionAnswer {
    Response(Box<ChatResponse>),
    Failure(Box<SpeechCallError>),
}

pub(super) struct ScriptedCompletions {
    answer: Mutex<Option<CompletionAnswer>>,
    calls: Mutex<Vec<IndexMap<String, Json>>>,
}

impl ScriptedCompletions {
    pub(super) fn answered(response: ChatResponse) -> Self {
        Self {
            answer: Mutex::new(Some(CompletionAnswer::Response(Box::new(response)))),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing(error: SpeechCallError) -> Self {
        Self {
            answer: Mutex::new(Some(CompletionAnswer::Failure(Box::new(error)))),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<IndexMap<String, Json>> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

impl SpeechCompletions for ScriptedCompletions {
    type Error = SpeechCallError;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, SpeechCallError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.kwargs);
        let answer = self
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let output = match answer {
            Some(CompletionAnswer::Response(response)) => Ok(SpeechCompletionOutput::Response(
                Box::new(SpeechCompletionAnswer {
                    response: *response,
                    hidden_params: HiddenParams::new(),
                }),
            )),
            Some(CompletionAnswer::Failure(error)) => Err(*error),
            None => Err(SpeechCallError::Internal {
                message: String::from("the completion ran twice"),
            }),
        };
        ready(output)
    }
}

pub(super) fn responded(model: &str, audio: Option<&str>) -> ChatResponse {
    let audio_part = audio.map_or_else(String::new, |data| {
        format!(r#", "audio": {{"data": "{data}"}}"#)
    });
    let body = format!(
        r#"{{"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": {model:?},
            "choices": [{{"index": 0, "message": {{"role": "assistant", "content": null{audio_part}}},
            "finish_reason": "stop"}}],
            "usage": {{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}}}}"#
    );
    serde_json::from_str(&body).expect("the provider response")
}

// one speech call over scripted seams: the settings, the transport and the case environment
pub(super) struct Harness<Transport> {
    fixture: Fixture,
    pub(super) settings: Settings,
    pub(super) transport: Transport,
    pub(super) completions: ScriptedCompletions,
    pub(super) configs: Configs,
    pub(super) operations: ScriptedOperations,
}

impl<Transport> Harness<Transport>
where
    Transport: SpeechTransport + Sync,
{
    pub(super) fn new(
        settings: Settings,
        transport: Transport,
        variables: &[(&str, &str)],
    ) -> Self {
        Self {
            fixture: Fixture::with_environment(variables),
            settings,
            transport,
            completions: ScriptedCompletions::failing(SpeechCallError::Internal {
                message: String::from("the bridge is not used"),
            }),
            configs: Configs::absent(),
            operations: ScriptedOperations::ok(b"unused"),
        }
    }

    pub(super) fn configs(mut self, configs: Configs) -> Self {
        self.configs = configs;
        self
    }

    pub(super) fn completions(mut self, completions: ScriptedCompletions) -> Self {
        self.completions = completions;
        self
    }

    pub(super) fn operations(mut self, operations: ScriptedOperations) -> Self {
        self.operations = operations;
        self
    }

    pub(super) fn run(&self, call: &SpeechCall) -> Result<BinaryContent, SpeechException> {
        let mut logging = logging_state();
        self.run_logged(call, &mut logging)
    }

    // a run whose logging object the test keeps, so it can read the call record and the pre-call hook
    pub(super) fn run_logged(
        &self,
        call: &SpeechCall,
        logging: &mut dyn SpeechCallLogging,
    ) -> Result<BinaryContent, SpeechException> {
        let context = self.fixture.context();
        let dispatch = SpeechDispatch {
            settings: &self.settings,
            resolution: &context,
            transport: &self.transport,
            completions: &self.completions,
            configs: &self.configs,
            operations: &self.operations,
        };
        runtime().block_on(dispatch.speech(call, logging, &redactor()))
    }

    pub(super) fn run_ok(&self, call: &SpeechCall) -> BinaryContent {
        self.run(call).expect("the speech call answers")
    }

    pub(super) fn run_err(&self, call: &SpeechCall) -> SpeechException {
        self.run(call).expect_err("the speech call fails")
    }
}
