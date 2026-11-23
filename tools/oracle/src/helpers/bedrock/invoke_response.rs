use betterlite_bedrock::agentcore_stream::UuidV4;
use betterlite_bedrock::converse_response::request::OptionalParams;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_response::{InvokeReplyCall, RawInvokeReply, decode_invoke_reply};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::request::Message;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, from_value, json, to_value};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod counter;
mod error;
mod fixture;
mod tests;
mod view;

use counter::{NoReasoning, TokenizerCounter};
use error::reply_error;
use fixture::{Silent, build_response, logging};
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
    messages: Json,
    request_data: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
    tool_name_mappings: IndexMap<String, String>,
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
                Err(error) => json!({
                    "error": {
                        "class": error.class,
                        "status": error.status,
                    }
                }),
            })
            .collect();
        Ok(json!({ "results": answers }))
    }
}

fn reply_case(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let sink = Silent;
    let (mut state, env) = logging(&redactor, &sink)?;
    let messages = to_value(&call.messages)?;
    let counted: Vec<Message> = from_value(messages.clone())?;
    let optional: OptionalParams = from_value(to_value(&call.optional_params)?)?;
    let decode_call = InvokeReplyCall {
        model: &call.model,
        messages: &call.messages,
        counted_messages: &counted,
        request_data: &call.request_data,
        optional_params: &call.optional_params,
        optional: &optional,
        litellm_params: &call.litellm_params,
        tool_name_mappings: &call.tool_name_mappings,
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
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    match runtime.block_on(decode_invoke_reply(
        decode_call,
        &raw,
        build_response(&call.response),
        &TokenizerCounter { messages },
        &NoReasoning,
    )) {
        Ok(reply) => Ok(reply_json(&reply)),
        Err(error) => Err(reply_error(&error)),
    }
}
