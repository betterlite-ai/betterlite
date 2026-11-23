#![no_main]

use std::convert::Infallible;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_bedrock::agentcore_stream::UuidV4;
use betterlite_bedrock::converse_response::request::OptionalParams;
use betterlite_bedrock::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_response::{InvokeReplyCall, RawInvokeReply, decode_invoke_reply};
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::{Builder, Runtime};

const INIT: &str = r#"{"model":"amazon.titan-text-express-v1","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"fuzz-call","function_id":"fuzz-function","litellm_trace_id":"fuzz-trace"}"#;
const SEED: &str = r#"{"id":"chatcmpl-fuzz","object":"chat.completion","created":0,"model":"","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":null,"annotations":null,"audio":null,"refusal":null},"finish_reason":null,"logprobs":null}],"system_fingerprint":null,"service_tier":null}"#;

struct ZeroCounter;

#[async_trait]
impl TokenCounter for ZeroCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }
}

struct NoReasoning;

impl AnthropicReasoningCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

impl ConverseReasoningCounter for NoReasoning {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}

struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging<'borrow>(
    redactor: &'borrow Redactor,
    sink: &'borrow DiscardingSink,
) -> Option<(LoggingState, PrePostEnv<'borrow>)> {
    let init: LoggingInit = serde_json::from_str(INIT).ok()?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("fuzz-trace"),
        Duration::default,
    )
    .ok()?;
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339("2026-01-01T00:00:05+00:00").ok()?,
    };
    Some((state, env))
}

fn replay(bytes: &[u8]) {
    let Ok(runtime) = Builder::new_current_thread().build() else {
        return;
    };
    let Ok(redactor) = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) else {
        return;
    };
    let sink = DiscardingSink;
    let Some((mut state, env)) = logging(&redactor, &sink) else {
        return;
    };
    let Ok(seed) = serde_json::from_str::<ChatResponse>(SEED) else {
        return;
    };
    decode(&runtime, &mut state, &env, bytes, seed);
}

fn decode(
    runtime: &Runtime,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    bytes: &[u8],
    seed: ChatResponse,
) {
    let params: IndexMap<String, Json> = IndexMap::new();
    let mappings: IndexMap<String, String> = IndexMap::new();
    let messages = Json::List(Vec::new());
    let optional = OptionalParams::default();
    let call = InvokeReplyCall {
        model: "amazon.titan-text-express-v1",
        messages: &messages,
        counted_messages: &[],
        request_data: &params,
        optional_params: &params,
        optional: &optional,
        litellm_params: &params,
        tool_name_mappings: &mappings,
        json_mode: None,
        disable_add_prefix_to_prompt: false,
        now: 0.0,
        ids: &UuidV4,
        log: LogContext { state, env },
    };
    let raw = RawInvokeReply {
        status: 200,
        headers: &[],
        body: bytes,
    };
    let _outcome = runtime.block_on(decode_invoke_reply(
        call,
        &raw,
        seed,
        &ZeroCounter,
        &NoReasoning,
    ));
}

fuzz_target!(|bytes: &[u8]| {
    replay(bytes);
});
