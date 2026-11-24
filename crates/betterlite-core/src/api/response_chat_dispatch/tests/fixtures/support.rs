#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
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
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_dispatch::error::PostStreamError;
use crate::api::response_chat_dispatch::input::{BridgeCall, ModelResponseSeed};
use crate::api::response_chat_dispatch::port::{
    Answer, PostStreamDeps, PostStreamProcessing, ResponsesCall, ResponsesEventStream,
    ResponsesExecution,
};

#[derive(Debug, Error)]
#[error("the fake execution failed")]
pub struct FakeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub model: String,
    pub provider: Option<String>,
    pub client: bool,
    pub params_stream: Option<Option<bool>>,
}

pub struct FakeExecution {
    answers: Mutex<VecDeque<Result<Answer, FakeError>>>,
    observations: Mutex<Vec<Observation>>,
}

impl FakeExecution {
    pub fn new(answer: Answer) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Ok(answer)])),
            observations: Mutex::new(Vec::new()),
        }
    }

    pub fn failing() -> Self {
        Self {
            answers: Mutex::new(VecDeque::from([Err(FakeError)])),
            observations: Mutex::new(Vec::new()),
        }
    }

    pub fn observations(&self) -> Vec<Observation> {
        self.observations.lock().unwrap().clone()
    }

    fn record(&self, call: &ResponsesCall<'_, u8>) -> Result<Answer, FakeError> {
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
        self.observations.lock().unwrap().push(Observation {
            model: call.params.model.clone(),
            provider,
            client: call.client.is_some(),
            params_stream: call.params.stream,
        });
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(FakeError))
    }
}

#[async_trait]
impl ResponsesExecution<u8> for FakeExecution {
    type Error = FakeError;

    async fn responses(&self, call: ResponsesCall<'_, u8>) -> Result<Answer, FakeError> {
        self.record(&call)
    }

    async fn aresponses(&self, call: ResponsesCall<'_, u8>) -> Result<Answer, FakeError> {
        self.record(&call)
    }
}

pub struct FakeEvents {
    lines: VecDeque<Result<Vec<u8>, StreamFailure>>,
    hidden: Map<String, Value>,
    dropped: Option<Arc<Mutex<bool>>>,
}

impl FakeEvents {
    pub fn new(lines: Vec<Vec<u8>>, hidden: Map<String, Value>) -> Self {
        Self {
            lines: lines.into_iter().map(Ok).collect(),
            hidden,
            dropped: None,
        }
    }

    pub fn failing(failure: StreamFailure) -> Self {
        Self {
            lines: VecDeque::from([Err(failure)]),
            hidden: Map::new(),
            dropped: None,
        }
    }

    pub fn with_drop_flag(mut self, flag: Arc<Mutex<bool>>) -> Self {
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

pub struct FakeChunks {
    chunks: VecDeque<Json>,
}

impl FakeChunks {
    pub fn new(chunks: &[&str]) -> Self {
        Self {
            chunks: chunks
                .iter()
                .map(|chunk| serde_json::from_str(chunk).unwrap())
                .collect(),
        }
    }
}

#[async_trait]
impl AsyncChunkStream for FakeChunks {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        Ok(self.chunks.pop_front())
    }
}

pub struct Probe;

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
pub struct FamilyByName;

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

pub struct NoJsonProviders;

impl JsonProviderProjection for NoJsonProviders {
    fn lookup(&self, _provider: ProviderId) -> JsonProviderLookup {
        JsonProviderLookup::Absent
    }
}

pub struct MissingJsonProviders;

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
pub struct RecordingProcessing {
    pub calls: Mutex<Vec<ChatConfigTag>>,
}

impl PostStreamProcessing for RecordingProcessing {
    fn post_stream_processing(
        &self,
        tag: ChatConfigTag,
        stream: Box<dyn AsyncChunkStream + Send>,
    ) -> Result<Box<dyn AsyncChunkStream + Send>, PostStreamError> {
        self.calls.lock().unwrap().push(tag);
        drop(stream);
        Ok(Box::new(FakeChunks::new(&["{\"replaced\":true}"])))
    }
}

pub fn post_stream<'deps>(processing: &'deps RecordingProcessing) -> PostStreamDeps<'deps> {
    deps(processing, &NO_JSON_PROVIDERS, &PROBE)
}

pub fn post_stream_missing_record<'deps>(
    processing: &'deps RecordingProcessing,
) -> PostStreamDeps<'deps> {
    deps(processing, &MISSING_JSON_PROVIDERS, &PROBE)
}

pub fn post_stream_named<'deps>(processing: &'deps RecordingProcessing) -> PostStreamDeps<'deps> {
    deps(processing, &NO_JSON_PROVIDERS, &FAMILY_BY_NAME)
}

fn deps<'deps>(
    processing: &'deps RecordingProcessing,
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

pub fn logging() -> LoggingState {
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
        dynamic_callbacks: Default::default(),
        applied_guardrails: None,
        request_logging: Default::default(),
        callback_policy: Default::default(),
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

pub fn kwargs(model: &str, provider: &str) -> IndexMap<String, Json> {
    let text = format!(
        r#"{{"model":{},"custom_llm_provider":{},"messages":[{{"role":"user","content":"hi"}}],"optional_params":{{}},"litellm_params":{{}},"headers":{{}},"model_response":{{}},"logging_obj":{{}}}}"#,
        Json::Text(model.to_owned()),
        Json::Text(provider.to_owned())
    );
    let Json::Object(fields) = serde_json::from_str(&text).unwrap() else {
        panic!("the fixture kwargs are an object");
    };
    fields
}

pub fn call<'call>(kwargs: &'call IndexMap<String, Json>) -> BridgeCall<'call, u8> {
    BridgeCall {
        kwargs,
        model_response: ModelResponseSeed {
            id: String::from("chatcmpl-seed"),
            created: 5,
            object: String::from("chat.completion"),
        },
        logging: logging(),
        client: Some(&7_u8),
        limits: ApiLimits::default(),
        reasoning_auto_summary: false,
        post_stream: None,
    }
}

pub fn sse(event: &str) -> Vec<u8> {
    format!("data: {event}").into_bytes()
}

pub fn hidden(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(key, value)| (String::from(*key), Value::from(*value)))
        .collect()
}

fn choice<'json>(chunk: &'json Json) -> Option<&'json IndexMap<String, Json>> {
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

pub fn text_of(chunk: &Json) -> Option<String> {
    let first = choice(chunk)?;
    let Json::Object(delta) = first.get("delta")? else {
        return None;
    };
    text_member(delta, "content")
}

pub fn finish_of(chunk: &Json) -> Option<String> {
    text_member(choice(chunk)?, "finish_reason")
}

pub fn id_of(chunk: &Json) -> Option<String> {
    let Json::Object(fields) = chunk else {
        return None;
    };
    text_member(fields, "id")
}

pub fn responses_body(text: &str) -> String {
    format!(
        r#"{{"id":"resp_a","created_at":1700000000,"status":"completed","output":[{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{text}","annotations":[]}}]}}]}}"#
    )
}

pub fn terminal_event(text: &str) -> String {
    format!(
        r#"{{"type":"response.completed","response":{}}}"#,
        responses_body(text)
    )
}
