#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::bedrock_config_selection::chat::BedrockRoute;
use betterlite_context::chat_config_selection::facts::{BedrockFacts, ModelFamilyProbe};
use betterlite_context::chat_config_selection::json_provider::{
    JsonProviderLookup, JsonProviderProjection,
};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_context::chat_config_selection::vertex::VertexModelLists;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::JsonObject;
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_dispatch::error::PostStreamError;
use crate::api::response_chat_dispatch::input::{BridgeCall, StreamStamps};
use crate::api::response_chat_dispatch::port::{
    Answer, BridgeStream, ChatChoice, ChatMessage, ChatResponse, PostStreamDeps,
    PostStreamProcessing, ResponsesCall, ResponsesEventStream, ResponsesExecution,
};
use crate::api::response_chat_result::records::ModelResponse;

#[derive(Debug, Error)]
#[error("the fake execution failed")]
pub(crate) struct FakeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamParam {
    Absent,
    Null,
    Bool(bool),
}

// Which entry the bridge took, so a test can tell responses() from aresponses()
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Responses,
    Aresponses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observation {
    pub model: String,
    pub provider: Option<String>,
    pub client: bool,
    pub params_stream: StreamParam,
    pub entry: Entry,
}

pub(crate) struct FakeExecution {
    answers: Mutex<VecDeque<Result<Answer, FakeError>>>,
    observations: Mutex<Vec<Observation>>,
}

impl FakeExecution {
    pub(crate) fn new(answer: Answer) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Ok(answer)])),
            observations: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Err(FakeError)])),
            observations: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn observations(&self) -> Vec<Observation> {
        self.observations.lock().unwrap().clone()
    }

    fn record(&self, call: &ResponsesCall<'_, u8>, entry: Entry) -> Result<Answer, FakeError> {
        let provider = match call.litellm_params.get("custom_llm_provider") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::Array(_)
                | Value::Object(_),
            )
            | None => None,
        };
        let params_stream = match call.params.stream {
            None => StreamParam::Absent,
            Some(None) => StreamParam::Null,
            Some(Some(stream)) => StreamParam::Bool(stream),
        };
        self.observations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Observation {
                model: call.params.model.clone(),
                provider,
                client: call.client.is_some(),
                params_stream,
                entry,
            });
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Err(FakeError))
    }
}

#[async_trait]
impl ResponsesExecution<u8> for FakeExecution {
    type Error = FakeError;

    async fn responses(&self, call: ResponsesCall<'_, u8>) -> Result<Answer, FakeError> {
        self.record(&call, Entry::Responses)
    }

    async fn aresponses(&self, call: ResponsesCall<'_, u8>) -> Result<Answer, FakeError> {
        self.record(&call, Entry::Aresponses)
    }
}

pub(crate) struct FakeEvents {
    lines: VecDeque<Result<Vec<u8>, StreamFailure>>,
    hidden: Map<String, Value>,
    dropped: Option<Arc<Mutex<bool>>>,
}

impl FakeEvents {
    pub(crate) fn new(lines: Vec<Vec<u8>>, hidden: Map<String, Value>) -> Self {
        Self {
            lines: lines.into_iter().map(Ok).collect(),
            hidden,
            dropped: None,
        }
    }

    pub(crate) fn failing(failure: StreamFailure) -> Self {
        Self {
            lines: VecDeque::from([Err(failure)]),
            hidden: Map::new(),
            dropped: None,
        }
    }

    pub(crate) fn with_drop_flag(mut self, flag: Arc<Mutex<bool>>) -> Self {
        self.dropped = Some(flag);
        self
    }
}

impl Drop for FakeEvents {
    fn drop(&mut self) {
        if let Some(flag) = &self.dropped {
            *flag.lock().unwrap() = true;
        }
    }
}

#[async_trait]
impl ResponsesEventStream for FakeEvents {
    async fn next_event(&mut self) -> Option<Result<Vec<u8>, StreamFailure>> {
        self.lines.pop_front()
    }

    fn hidden_params(&self) -> &Map<String, Value> {
        &self.hidden
    }
}

pub(crate) struct FakeChunks {
    chunks: VecDeque<Json>,
    reads: Option<Arc<Mutex<Vec<Option<Json>>>>>,
}

impl FakeChunks {
    pub(crate) fn new(chunks: &[&str]) -> Self {
        Self {
            chunks: chunks
                .iter()
                .map(|chunk| serde_json::from_str(chunk).unwrap())
                .collect(),
            reads: None,
        }
    }

    // every read, the closing None included, lands in the caller's log
    pub(crate) fn tracking(chunks: &[&str], reads: &Arc<Mutex<Vec<Option<Json>>>>) -> Self {
        Self {
            reads: Some(Arc::clone(reads)),
            ..Self::new(chunks)
        }
    }
}

#[async_trait]
impl AsyncChunkStream for FakeChunks {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        let chunk = self.chunks.pop_front();
        if let Some(reads) = &self.reads {
            reads.lock().unwrap().push(chunk.clone());
        }
        Ok(chunk)
    }
}

pub(crate) struct Probe;

impl ModelFamilyProbe for Probe {
    fn openai_o_series(&self, _model: &str) -> bool {
        false
    }

    fn openai_gpt_5(&self, _model: &str) -> bool {
        false
    }

    fn openai_catalog_model(&self, _model: &str) -> bool {
        true
    }

    fn azure_gpt_5(&self, _model: &str) -> bool {
        false
    }
}

// a probe whose answers depend on the exact model text, so a routing prefix would change the selection
pub(crate) struct FamilyByName;

impl ModelFamilyProbe for FamilyByName {
    fn openai_o_series(&self, model: &str) -> bool {
        model == "o1"
    }

    fn openai_gpt_5(&self, model: &str) -> bool {
        model == "gpt-5"
    }

    fn openai_catalog_model(&self, model: &str) -> bool {
        model == "gpt-4o-mini"
    }

    fn azure_gpt_5(&self, model: &str) -> bool {
        model == "gpt-5"
    }
}

pub(crate) struct NoJsonProviders;

impl JsonProviderProjection for NoJsonProviders {
    fn lookup(&self, _provider: ProviderId) -> JsonProviderLookup {
        JsonProviderLookup::Absent
    }
}

pub(crate) struct MissingJsonProviders;

impl JsonProviderProjection for MissingJsonProviders {
    fn lookup(&self, _provider: ProviderId) -> JsonProviderLookup {
        JsonProviderLookup::MissingRecord
    }
}

static PROBE: Probe = Probe;
static FAMILY_BY_NAME: FamilyByName = FamilyByName;
static NO_JSON_PROVIDERS: NoJsonProviders = NoJsonProviders;
static MISSING_JSON_PROVIDERS: MissingJsonProviders = MissingJsonProviders;

#[derive(Default)]
pub(crate) struct RecordingProcessing {
    pub calls: Mutex<Vec<ChatConfigTag>>,
}

impl PostStreamProcessing for RecordingProcessing {
    fn post_stream_processing(
        &self,
        tag: ChatConfigTag,
        stream: BridgeStream,
    ) -> Result<BridgeStream, PostStreamError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tag);
        drop(stream);
        Ok(BridgeStream::Wire(Box::new(FakeChunks::new(&[
            "{\"replaced\":true}",
        ]))))
    }
}

// The provider config with no reshaping of its own: whatever the bridge produced is delivered
#[derive(Default)]
pub(crate) struct PassThroughProcessing;

impl PostStreamProcessing for PassThroughProcessing {
    fn post_stream_processing(
        &self,
        _tag: ChatConfigTag,
        stream: BridgeStream,
    ) -> Result<BridgeStream, PostStreamError> {
        Ok(stream)
    }
}

pub(crate) fn post_stream(processing: &RecordingProcessing) -> PostStreamDeps<'_> {
    deps(processing, &NO_JSON_PROVIDERS, &PROBE)
}

pub(crate) fn post_stream_missing_record(processing: &RecordingProcessing) -> PostStreamDeps<'_> {
    deps(processing, &MISSING_JSON_PROVIDERS, &PROBE)
}

pub(crate) fn post_stream_named(processing: &RecordingProcessing) -> PostStreamDeps<'_> {
    deps(processing, &NO_JSON_PROVIDERS, &FAMILY_BY_NAME)
}

pub(crate) fn post_stream_passthrough(processing: &PassThroughProcessing) -> PostStreamDeps<'_> {
    deps(processing, &NO_JSON_PROVIDERS, &FAMILY_BY_NAME)
}

fn deps<'deps>(
    processing: &'deps dyn PostStreamProcessing,
    json_providers: &'deps (dyn JsonProviderProjection + Send + Sync),
    families: &'deps (dyn ModelFamilyProbe + Send + Sync),
) -> PostStreamDeps<'deps> {
    PostStreamDeps {
        families,
        base_model: None,
        bedrock: BedrockFacts {
            route: BedrockRoute::ChatCompletions,
            invoke_provider: None,
            base_model: "",
        },
        vertex: VertexModelLists::default(),
        json_providers,
        processing,
    }
}

pub(crate) fn logging() -> LoggingState {
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let init = LoggingInit {
        model: String::from("gpt-4o-mini"),
        call_type: String::from("completion"),
        start_time: DateTime::UNIX_EPOCH.fixed_offset(),
        litellm_call_id: String::from("call-1"),
        function_id: String::from("fn-1"),
        messages: None,
        stream: None,
        litellm_trace_id: Some(String::from("trace-1")),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated-trace"),
        || Duration::ZERO,
    )
    .unwrap()
}

pub(crate) fn kwargs(model: &str, provider: &str) -> IndexMap<String, Json> {
    let text = format!(
        r#"{{"model":"{model}","custom_llm_provider":"{provider}","messages":[{{"role":"user","content":"hi"}}],"optional_params":{{}},"litellm_params":{{}},"headers":{{}},"model_response":{{}},"logging_obj":{{}}}}"#
    );
    let Json::Object(fields) = serde_json::from_str(&text).unwrap() else {
        panic!("the fixture kwargs are an object");
    };
    fields
}

// The caller's record as the SDK boundary builds it: the answer keeps these three fields' values on collection
pub(crate) fn caller_response() -> ModelResponse {
    ModelResponse {
        id: String::from("chatcmpl-seed"),
        created: 5,
        model: None,
        object: String::from("chat.completion"),
        system_fingerprint: None,
        choices: Vec::new(),
        usage: None,
        extra: Map::new(),
        hidden_params: Map::new(),
    }
}

// The caller's record with the fingerprint, extra field and headers a collected answer must keep
pub(crate) fn caller_with_identity_extras() -> ModelResponse {
    let mut caller = caller_response();
    caller.system_fingerprint = Some(String::from("fp-1"));
    caller
        .extra
        .insert(String::from("trace"), Value::from("t-1"));
    caller.hidden_params.insert(
        String::from("additional_headers"),
        serde_json::from_str(r#"{"shared":"caller","caller_only":"c"}"#).unwrap(),
    );
    caller
}

// A whole chat answer, as a provider without native streaming returns one
pub(crate) fn chat_response() -> Box<ChatResponse> {
    Box::new(ChatResponse {
        id: String::from("chatcmpl-x"),
        object: String::from("chat.completion"),
        created: 1_700_000_000,
        model: Some(String::from("gpt-4o-mini")),
        system_fingerprint: None,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                content: Some(String::from("hello")),
                role: Some(String::from("assistant")),
                function_call: None,
                tool_calls: None,
                audio: None,
                reasoning_content: None,
                thinking_blocks: None,
                images: None,
                reasoning_items: None,
                annotations: None,
                provider_specific_fields: None,
                extra: JsonObject::new(),
            },
            finish_reason: Some(String::from("stop")),
            logprobs: None,
            provider_specific_fields: None,
            extra: JsonObject::new(),
        }],
        usage: None,
        extra: JsonObject::new(),
    })
}

// The typed message derives no equality, so a pass-through test compares the JSON both answers dump
pub(crate) fn chat_json(response: &ChatResponse) -> String {
    serde_json::to_string(response).unwrap()
}

pub(crate) const FIXED_UUID: &str = "00000000-0000-4000-8000-000000000001";
pub(crate) const FIXED_CLOCK: i64 = 41;
pub(crate) const FIXED_TIMESTAMP: f64 = 41.5;

fn fixed_next_id() -> Uuid {
    Uuid::parse_str(FIXED_UUID).unwrap()
}

const fn fixed_clock() -> i64 {
    FIXED_CLOCK
}

const fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: FIXED_CLOCK,
        timestamp: FIXED_TIMESTAMP,
    }
}

// The clock and identifier sources every bridge call gets, so a test can name the id its stream invents
pub(crate) fn stamps() -> StreamStamps {
    StreamStamps {
        now: fixed_instant,
        next_id: fixed_next_id,
        clock: fixed_clock,
    }
}

pub(crate) fn call(kwargs: &IndexMap<String, Json>) -> BridgeCall<'_, u8> {
    BridgeCall {
        kwargs,
        model_response: caller_response(),
        logging: logging(),
        client: Some(&7_u8),
        limits: ApiLimits::default(),
        reasoning_auto_summary: false,
        post_stream: None,
        stamps: stamps(),
    }
}

pub(crate) fn sse(event: &str) -> Vec<u8> {
    format!("data: {event}").into_bytes()
}

pub(crate) fn hidden(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| (String::from(*key), Value::from(*value)))
        .collect()
}

fn choice(chunk: &Json) -> Option<&IndexMap<String, Json>> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    let Json::List(choices) = fields.get("choices")? else {
        return None;
    };
    let Json::Object(item) = choices.first()? else {
        return None;
    };
    Some(item)
}

fn text_member(fields: &IndexMap<String, Json>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(crate) fn text_of(chunk: &Json) -> Option<String> {
    let first = choice(chunk)?;
    let Json::Object(delta) = first.get("delta")? else {
        return None;
    };
    text_member(delta, "content")
}

pub(crate) fn finish_of(chunk: &Json) -> Option<String> {
    text_member(choice(chunk)?, "finish_reason")
}

pub(crate) fn id_of(chunk: &Json) -> Option<String> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    text_member(fields, "id")
}

// A member of the first choice's delta, such as its tool calls or the thinking blocks a replay carried
pub(crate) fn delta_member_of(chunk: &Json, name: &str) -> Option<Json> {
    let Json::Object(delta) = choice(chunk)?.get("delta")? else {
        return None;
    };
    delta.get(name).cloned()
}

// A member the wrapper folded into the private map, such as its running usage or its created_at
pub(crate) fn hidden_member_of(chunk: &Json, name: &str) -> Option<Json> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    let Json::Object(hidden) = fields.get("_hidden_params")? else {
        return None;
    };
    hidden.get(name).cloned()
}

pub(crate) fn responses_body(text: &str) -> String {
    format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"completed","output":[{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{text}","annotations":[]}}]}}]}}"#
    )
}

pub(crate) fn terminal_event(text: &str) -> String {
    format!(
        r#"{{"type":"response.completed","response":{}}}"#,
        responses_body(text)
    )
}
