#![cfg(test)]

use std::future::Future;
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
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::request::Message;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use tokio::runtime::Builder;

use crate::agentcore_stream::UuidV4;
use crate::bedrock_settings::BedrockError;
use crate::converse_response::request::OptionalParams;
use crate::image_call::plan::LogContext;
use crate::invoke_primitives::config::Params;
use crate::invoke_response::{InvokeReplyCall, RawInvokeReply};
use crate::qwen_deepseek::error::{DeepseekReplyError, QwenReplyError};
use crate::qwen_deepseek::reply::Reply;
use crate::qwen_deepseek::{QwenProjection, decode_deepseek_reply, transform_qwen_response};

use super::counter::{CounterCall, FixtureCounter, NoReasoning};

pub(super) const MODEL: &str = "qwen.qwen3-32b-v1:0";
pub(super) const NOW: f64 = 1_700_000_000.0;
pub(super) const CREATED: u64 = 1_700_000_000;
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
    pub(super) response: Reply,
    pub(super) prompt_tokens: i64,
    pub(super) completion_tokens: i64,
}

pub(super) fn fixture() -> Fixture {
    Fixture {
        model: MODEL.to_owned(),
        messages: json(r#"[{"role": "user", "content": "hi"}]"#),
        counted_messages: serde_json::from_str(r#"[{"role": "user", "content": "hi"}]"#)
            .expect("the counted messages parse"),
        request_data: object(r#"{"model": "qwen.qwen3-32b-v1:0"}"#),
        optional_params: IndexMap::new(),
        litellm_params: IndexMap::new(),
        optional: OptionalParams::default(),
        tool_name_mappings: IndexMap::new(),
        json_mode: None,
        disable_add_prefix_to_prompt: false,
        now: NOW,
        status: 200,
        headers: Vec::new(),
        response: seeded(),
        prompt_tokens: 7,
        completion_tokens: 3,
    }
}

// The DeepSeek base conversion only reads the generation family, so the model names that provider.
pub(super) fn deepseek_fixture() -> Fixture {
    let mut case = fixture();
    case.model = String::from("deepseek_r1.deepseek-r1-v1:0");
    case
}

pub(super) fn seeded() -> Reply {
    serde_json::from_str(
        r#"{"id":"chatcmpl-supplied","object":"chat.completion","created":1,"model":"previous-model","choices":[{"index":0,"message":{"role":"assistant","content":null},"finish_reason":null}]}"#,
    )
    .expect("the seeded reply parses")
}

pub(super) fn two_choices() -> Reply {
    serde_json::from_str(
        r#"{"id":"chatcmpl-supplied","object":"chat.completion","created":1,"model":"previous-model","usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3},"choices":[{"index":0,"message":{"role":"assistant","content":"old"},"finish_reason":"length"},{"index":1,"message":{"role":"assistant","content":"second"},"finish_reason":"length"}]}"#,
    )
    .expect("the seeded reply parses")
}

pub(super) fn with_tool_calls(mut reply: Reply) -> Reply {
    reply.choices[0].message.tool_calls = Some(vec![tool_call()]);
    reply
}

pub(super) fn tool_call() -> ToolCall {
    serde_json::from_str(
        r#"{"id": "call-1", "type": "function", "function": {"name": "read", "arguments": "{}"}}"#,
    )
    .expect("the tool call fixture parses")
}

pub(super) struct QwenRun {
    pub(super) reply: Result<Reply, QwenReplyError>,
    pub(super) details: IndexMap<String, Json>,
}

impl QwenRun {
    pub(super) fn reply(&self) -> &Reply {
        match &self.reply {
            Ok(reply) => reply,
            Err(error) => panic!("the Qwen reply failed: {error}"),
        }
    }

    pub(super) fn error(&self) -> &QwenReplyError {
        match &self.reply {
            Ok(_reply) => panic!("the Qwen reply unexpectedly succeeded"),
            Err(error) => error,
        }
    }

    // A failure is logged when post_call recorded its four fields; a success records none of them.
    pub(super) fn logged(&self) -> bool {
        self.details.contains_key("log_event_type")
    }

    pub(super) fn logged_response(&self) -> &Json {
        self.details
            .get("original_response")
            .expect("the failure logged its original_response")
    }

    pub(super) fn logged_input(&self) -> &Json {
        self.details
            .get("input")
            .expect("the failure logged its input")
    }

    pub(super) fn logged_error(&self) -> &str {
        let Some(Json::Object(fields)) = self.details.get("additional_args") else {
            panic!("the failure logged no additional_args object");
        };
        match fields.get("error") {
            Some(Json::Text(text)) => text,
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => panic!("the failure logged no error text"),
        }
    }
}

pub(super) fn qwen(fixture: &Fixture, projection: QwenProjection, body: &str) -> QwenRun {
    qwen_bytes(fixture, projection, body.as_bytes())
}

// The invoke call and raw reply every reply test hands to the transform under test.
fn invoke_call<'call, 'log, 'env>(
    fixture: &'call Fixture,
    body: &'call [u8],
    state: &'log mut LoggingState,
    env: &'log PrePostEnv<'env>,
) -> (InvokeReplyCall<'call, 'log, 'env>, RawInvokeReply<'call>) {
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
        ids: &UuidV4,
        log: LogContext { state, env },
    };
    let raw = RawInvokeReply {
        status: fixture.status,
        headers: &fixture.headers,
        body,
    };
    (call, raw)
}

pub(super) fn qwen_bytes(fixture: &Fixture, projection: QwenProjection, body: &[u8]) -> QwenRun {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let sink = DiscardingSink;
    let (mut state, env) = logging(&redactor, &sink);
    let (call, raw) = invoke_call(fixture, body, &mut state, &env);
    let reply = transform_qwen_response(projection, call, &raw, fixture.response.clone());
    QwenRun {
        reply,
        details: state.model_call_details.clone(),
    }
}

pub(super) struct DeepseekRun {
    pub(super) reply: Result<Reply, DeepseekReplyError>,
    pub(super) calls: Vec<CounterCall>,
}

impl DeepseekRun {
    pub(super) fn reply(&self) -> &Reply {
        match &self.reply {
            Ok(reply) => reply,
            Err(error) => panic!("the DeepSeek reply failed: {error}"),
        }
    }

    pub(super) fn error(&self) -> &DeepseekReplyError {
        match &self.reply {
            Ok(_reply) => panic!("the DeepSeek reply unexpectedly succeeded"),
            Err(error) => error,
        }
    }

    pub(super) fn bedrock(&self) -> &BedrockError {
        match self.error() {
            DeepseekReplyError::Base(error) => error
                .bedrock()
                .expect("the base failure carries a Bedrock error"),
            other => panic!("the failure is not a base failure: {other}"),
        }
    }
}

pub(super) fn deepseek(fixture: &Fixture, body: &str) -> DeepseekRun {
    let counter = FixtureCounter::counting(fixture.prompt_tokens, fixture.completion_tokens);
    deepseek_with(fixture, body.as_bytes(), &counter)
}

pub(super) fn deepseek_with(
    fixture: &Fixture,
    body: &[u8],
    counter: &FixtureCounter,
) -> DeepseekRun {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds");
    let sink = DiscardingSink;
    let (mut state, env) = logging(&redactor, &sink);
    let (call, raw) = invoke_call(fixture, body, &mut state, &env);
    let reply = block_on(decode_deepseek_reply(
        call,
        &raw,
        fixture.response.clone(),
        counter,
        &NoReasoning,
    ));
    DeepseekRun {
        reply,
        calls: counter.calls(),
    }
}

fn logging<'log>(
    redactor: &'log Redactor,
    sink: &'log DiscardingSink,
) -> (LoggingState, PrePostEnv<'log>) {
    let init: LoggingInit = serde_json::from_str(INIT).expect("the logging fixture is valid");
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging fixture builds");
    let env = PrePostEnv {
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(LOG_TIME).expect("the clock parses"),
    };
    (state, env)
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

pub(super) fn prompts(source: &str) -> Vec<PromptJson> {
    serde_json::from_str(source).expect("the prompt messages parse")
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the params parse")
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
