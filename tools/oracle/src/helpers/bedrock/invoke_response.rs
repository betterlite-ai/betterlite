use betterlite_bedrock::agentcore_stream::UuidV4;
use betterlite_bedrock::converse_response::request::OptionalParams;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_response::{InvokeReplyCall, RawInvokeReply, decode_invoke_reply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod counter;
mod error;
mod fixture;
mod tests;
mod view;

use counter::{NoReasoning, TokenizerCounter};
use error::{error_json, reply_error, value_error};
use fixture::{Silent, build_response, deserialize, logging};
use view::reply_json;

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

fn reply_case(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| value_error(error.to_string()))?;
    let sink = Silent;
    let (mut state, env) = logging(&redactor, &sink)?;
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
