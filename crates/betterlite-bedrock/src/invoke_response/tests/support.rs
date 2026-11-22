#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::request::Message;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::converse_response::request::OptionalParams;
use crate::image_call::plan::LogContext;
use crate::invoke_response::{InvokeReplyCall, RawInvokeReply, decode_invoke_reply};

use super::counter::{FixedIds, FixtureCounter, NoReasoning};
use super::run::Run;

pub(super) const MODEL: &str = "amazon.titan-text-express-v1";
pub(super) const NOW: f64 = 1_700_000_000.0;
pub(super) const CREATED: u64 = 1_700_000_000;
const RESPONSE_OBJECT: &str = "chat.completion";
const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const LOG_TIME: &str = "2026-01-01T00:00:05+00:00";

pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) messages: Json,
    pub(super) counted_messages: Vec<Message>,
    pub(super) request_data: IndexMap<String, Json>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) optional: OptionalParams,
    pub(super) tool_name_mappings: IndexMap<String, String>,
    pub(super) json_mode: Option<bool>,
    pub(super) disable_add_prefix_to_prompt: bool,
    pub(super) now: f64,
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) ids: Vec<Uuid>,
    pub(super) response: ChatResponse,
    pub(super) prompt_tokens: i64,
    pub(super) completion_tokens: i64,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        model: MODEL.to_owned(),
        messages: json(r#"[{"role": "user", "content": "hi"}]"#),
        counted_messages: Vec::new(),
        request_data: object(r#"{"model": "amazon.titan-text-express-v1"}"#),
        optional_params: IndexMap::new(),
        litellm_params: IndexMap::new(),
        optional: OptionalParams::default(),
        tool_name_mappings: IndexMap::new(),
        json_mode: None,
        disable_add_prefix_to_prompt: false,
        now: NOW,
        status: 200,
        headers: Vec::new(),
        ids: Vec::new(),
        response: seeded(),
        prompt_tokens: 7,
        completion_tokens: 3,
    }
}

pub(super) fn seeded() -> ChatResponse {
    ChatResponse {
        id: String::from("chatcmpl-supplied"),
        object: RESPONSE_OBJECT.to_owned(),
        created: 1,
        model: String::from("previous-model"),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: None,
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

pub(super) fn with_tool_calls(
    mut response: ChatResponse,
    tool_calls: Vec<ToolCall>,
) -> ChatResponse {
    response.choices[0].message.tool_calls = Some(tool_calls);
    response
}

pub(super) fn tool_call() -> ToolCall {
    serde_json::from_str(
        r#"{"id": "call-1", "type": "function", "function": {"name": "read", "arguments": "{}"}}"#,
    )
    .expect("the tool call fixture parses")
}

pub(super) fn decode(fixture: &Fixture, body: &str) -> Run {
    let counter = FixtureCounter::counting(fixture.prompt_tokens, fixture.completion_tokens);
    decode_with(fixture, body.as_bytes(), &counter)
}

pub(super) fn decode_with(fixture: &Fixture, body: &[u8], counter: &FixtureCounter) -> Run {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let sink = DiscardingSink;
    let init: LoggingInit = serde_json::from_str(INIT).expect("the logging fixture is valid");
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .expect("the logging fixture builds");
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(LOG_TIME).expect("the clock parses"),
    };
    let ids = FixedIds {
        values: Mutex::new(fixture.ids.clone()),
    };
    let call = InvokeReplyCall {
        model: &fixture.model,
        messages: &fixture.messages,
        counted_messages: &fixture.counted_messages,
        request_data: &fixture.request_data,
        optional_params: &fixture.optional_params,
        optional: &fixture.optional,
        litellm_params: &fixture.litellm_params,
        tool_name_mappings: &fixture.tool_name_mappings,
        json_mode: fixture.json_mode,
        disable_add_prefix_to_prompt: fixture.disable_add_prefix_to_prompt,
        now: fixture.now,
        ids: &ids,
        log: LogContext {
            state: &mut state,
            env: &env,
        },
    };
    let raw = RawInvokeReply {
        status: fixture.status,
        headers: &fixture.headers,
        body,
    };
    let reply = block_on(decode_invoke_reply(
        call,
        &raw,
        fixture.response.clone(),
        counter,
        &NoReasoning,
    ));
    Run {
        reply,
        calls: counter.calls(),
    }
}

struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the test runtime could not start");
    runtime.block_on(future)
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

pub(super) fn object(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the fixture is not an object")
        }
    }
}

pub(super) fn headers(input: &str, output: &str) -> Vec<(String, String)> {
    vec![
        (
            "x-amzn-bedrock-input-token-count".to_owned(),
            input.to_owned(),
        ),
        (
            "x-amzn-bedrock-output-token-count".to_owned(),
            output.to_owned(),
        ),
    ]
}
