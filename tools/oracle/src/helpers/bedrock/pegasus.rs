use async_trait::async_trait;
use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_bedrock::pegasus::error::PegasusError;
use betterlite_bedrock::pegasus::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_bedrock::pegasus::prompt::PromptMessage;
use betterlite_bedrock::pegasus::reply::{ReplyContext, decode_pegasus_reply};
use betterlite_bedrock::pegasus::request::transform_request;
use betterlite_bedrock::pegasus::response::PegasusResponse;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    PegasusMediaSource(RequestCall),
    TextVideoPrompt(RequestCall),
    ResponseFormatJson(MappingCall),
    SegmentsAndResponse(ReplyCall),
    UsageAndFinishReason(ReplyCall),
    PegasusOptions(MappingCall),
    PegasusReplyErrors(ReplyCall),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RequestCall {
    messages: Vec<PromptMessage>,
    optional_params: Params,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MappingCall {
    non_default: Params,
    optional_params: Params,
    messages: Vec<PromptMessage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ReplyCall {
    body: String,
    status: u16,
    headers: Vec<(String, String)>,
    model: String,
    messages: Vec<Message>,
    now: f64,
    response: PegasusResponse,
}

struct UnusedCounter;

#[async_trait]
impl TokenCounter for UnusedCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }
}

impl From<PegasusError> for Box<HelperError> {
    fn from(error: PegasusError) -> Self {
        let (class, status) = match &error {
            PegasusError::Bedrock { status, .. } => ("BedrockError", Some(*status)),
            PegasusError::Header(_) => ("HeaderError", None),
            _ => ("ValueError", None),
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PegasusMediaSource(call) | Input::TextVideoPrompt(call) => request_case(call),
            Input::ResponseFormatJson(call) | Input::PegasusOptions(call) => mapping_case(call),
            Input::SegmentsAndResponse(call)
            | Input::UsageAndFinishReason(call)
            | Input::PegasusReplyErrors(call) => reply_case(call),
        }
    }
}

fn request_case(call: &RequestCall) -> HelperResult {
    let request = transform_request(&call.messages, &call.optional_params)?;
    Ok(json!({"request": serde_json::to_value(request)?}))
}

fn mapping_case(call: &MappingCall) -> HelperResult {
    let mapped = map_openai_params(&call.non_default, call.optional_params.clone())?;
    let request = transform_request(&call.messages, &mapped)?;
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": serde_json::to_value(&mapped)?,
        "request": serde_json::to_value(&request)?,
    }))
}

fn reply_case(call: &ReplyCall) -> HelperResult {
    let mut response = call.response.clone();
    let context = ReplyContext {
        status: call.status,
        headers: &call.headers,
        model: &call.model,
        messages: &call.messages,
        now: call.now,
    };
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    runtime.block_on(decode_pegasus_reply(
        call.body.as_bytes(),
        &context,
        &mut response,
        &UnusedCounter,
    ))?;
    Ok(response_json(&response))
}

fn response_json(response: &PegasusResponse) -> Value {
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "role": choice.message.role,
                "content": choice.message.content,
            })
        })
        .collect();
    json!({
        "id": response.id,
        "created": response.created,
        "model": response.model,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "choices": choices,
    })
}
