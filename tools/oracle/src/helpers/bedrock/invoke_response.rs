use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_bedrock::agentcore_stream::UuidV4;
use betterlite_bedrock::converse_response::request::OptionalParams;
use betterlite_bedrock::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_response::error::InvokeReplyError;
use betterlite_bedrock::invoke_response::{
    InvokeReply, InvokeReplyCall, RawInvokeReply, decode_invoke_reply,
};
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use chrono::{DateTime, FixedOffset};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_oracle_runner::helpers::{HelperCall, invoke};

mod tests;

const INIT: &str = r#"{"model":"fixture-model","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const LOG_TIME: &str = "2026-01-01T00:00:05+00:00";

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    reply: Vec<ReplyCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    model: String,
    body: String,
    status: u16,
    headers: Vec<(String, String)>,
    messages: Vec<Value>,
    request_data: Value,
    optional_params: Value,
    litellm_params: Value,
    tool_name_mappings: Value,
    json_mode: Option<bool>,
    now: f64,
    response: ResponseSeed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseSeed {
    id: String,
    choices: Vec<ChoiceSeed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChoiceSeed {
    index: u32,
    finish_reason: Option<String>,
    message: MessageSeed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageSeed {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<Value>>,
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
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

// The oracle answer for a fallback count is the token_count helper, so no tokenizer runs twice here.
struct TokenizerCounter {
    messages: Vec<Value>,
}

#[async_trait]
impl TokenCounter for TokenizerCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        count(json!({
            "function": "token_counter",
            "model": "",
            "messages": self.messages,
        }))
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        count(json!({
            "function": "token_counter",
            "model": "",
            "text": text,
            "count_response_tokens": true,
        }))
    }
}

// The token_count helper starts its own runtime, so the call runs off the runtime this decode drives.
fn count(input: Value) -> Result<i64, CountError> {
    let groups = [betterlite_oracle_tokenizer::GROUP];
    let counted = std::thread::spawn(move || {
        invoke(
            &groups,
            &HelperCall {
                operation: String::from("token_count"),
                input,
            },
        )
    })
    .join()
    .map_err(|_panic| CountError::MissingInput)?;
    let Some(value) = counted.value else {
        return Err(CountError::MissingInput);
    };
    serde_json::from_value(value).map_err(|_error| CountError::MissingInput)
}

fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn reply_error(error: &InvokeReplyError) -> Box<HelperError> {
    let Some(bedrock) = error.bedrock() else {
        return value_error(error.to_string());
    };
    Box::new(HelperError {
        class: String::from("BedrockError"),
        status: Some(bedrock.error.status_code),
        error_type: None,
        param: None,
        code: None,
        message: bedrock.error.message.clone(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers: Vec<Value> = input
            .reply
            .iter()
            .map(|call| match reply_case(call) {
                Ok(view) => json!({ "ok": view }),
                Err(error) => error_json(&error),
            })
            .collect();
        Ok(json!({ "results": answers }))
    }
}

fn error_json(error: &HelperError) -> Value {
    json!({
        "error": {
            "class": error.class,
            "status": error.status,
        }
    })
}

fn reply_case(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| value_error(error.to_string()))?;
    let sink = Silent;
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT)
            .map_err(|error| value_error(error.to_string()))?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| value_error(error.to_string()))?;
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
        now: DateTime::<FixedOffset>::parse_from_rfc3339(LOG_TIME)
            .map_err(|error| value_error(error.to_string()))?,
    };
    let counted = deserialize(&Value::Array(call.messages.clone()))?;
    let messages = deserialize(&Value::Array(call.messages.clone()))?;
    let request_data = deserialize(&call.request_data)?;
    let optional_params = deserialize(&call.optional_params)?;
    let litellm_params = deserialize(&call.litellm_params)?;
    let tool_name_mappings = deserialize(&call.tool_name_mappings)?;
    let optional = OptionalParams::default();
    let counter = TokenizerCounter {
        messages: call.messages.clone(),
    };
    let decode_call = InvokeReplyCall {
        model: &call.model,
        messages: &messages,
        counted_messages: &counted,
        request_data: &request_data,
        optional_params: &optional_params,
        optional: &optional,
        litellm_params: &litellm_params,
        tool_name_mappings: &tool_name_mappings,
        json_mode: call.json_mode,
        disable_add_prefix_to_prompt: false,
        now: call.now,
        ids: &UuidV4,
        log: LogContext {
            state: &mut state,
            env: &env,
        },
    };
    let raw = RawInvokeReply {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
    };
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| value_error(error.to_string()))?;
    match runtime.block_on(decode_invoke_reply(
        decode_call,
        &raw,
        build_response(&call.response),
        &counter,
        &NoReasoning,
    )) {
        Ok(reply) => Ok(reply_json(&reply)),
        Err(error) => Err(reply_error(&error)),
    }
}

fn deserialize<T>(value: &Value) -> Result<T, Box<HelperError>>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone()).map_err(|error| value_error(error.to_string()))
}

fn build_response(seed: &ResponseSeed) -> ChatResponse {
    let choices = seed
        .choices
        .iter()
        .map(|choice| ChatChoice {
            index: choice.index,
            message: AssistantMessage {
                role: choice.message.role.clone(),
                content: choice.message.content.clone(),
                tool_calls: choice.message.tool_calls.as_ref().map(|_calls| Vec::new()),
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: choice.finish_reason.clone(),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        })
        .collect();
    ChatResponse {
        id: seed.id.clone(),
        object: String::from("chat.completion"),
        created: 0,
        model: String::new(),
        choices,
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn usage_json<P, C, T>(prompt: P, completion: C, total: T) -> Value
where
    P: Serialize,
    C: Serialize,
    T: Serialize,
{
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}

fn reply_json(reply: &InvokeReply) -> Value {
    let (id, created, model, usage, choices) = match reply {
        InvokeReply::Chat(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            usage_json(
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                response.usage.total_tokens,
            ),
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Anthropic(envelope) => (
            envelope.id.as_str(),
            json!(envelope.created),
            json!(envelope.model),
            match &envelope.usage {
                Some(usage) => usage_json(
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ),
                None => Value::Null,
            },
            envelope
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Nova(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            match &response.usage {
                Some(usage) => usage_json(
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ),
                None => Value::Null,
            },
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
        InvokeReply::Pegasus(response) => (
            response.id.as_str(),
            json!(response.created),
            json!(response.model),
            usage_json(
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
                response.usage.total_tokens,
            ),
            response
                .choices
                .iter()
                .map(|choice| {
                    json!({
                        "index": choice.index,
                        "finish_reason": choice.finish_reason,
                        "content": choice.message.content,
                        "tool_calls": choice.message.tool_calls.as_ref().map(Vec::len),
                    })
                })
                .collect(),
        ),
    };
    json!({
        "id": id,
        "created": created,
        "model": model,
        "usage": usage,
        "choices": choices,
    })
}
