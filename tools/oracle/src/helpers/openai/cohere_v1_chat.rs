use betterlite_openai::cohere_chat_v1::error::CohereV1ConfigError;
use betterlite_openai::cohere_chat_v1::reply::{CohereV1Reply, ReplyInput, decode_cohere_chat_v1};
use betterlite_openai::cohere_chat_v1::{
    CohereV1ChatConfig, RequestInput, SUPPORTED_OPENAI_PARAMS, map_openai_params, request_options,
};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Options {
        // the pinned get_supported_openai_params returns the same list for any model
        #[serde(rename = "supported_model")]
        _supported_model: String,
        #[serde(default)]
        mappings: Vec<Mapping>,
    },
    Request {
        #[serde(default)]
        requests: Vec<RequestShape>,
    },
    Reply {
        #[serde(default)]
        requests: Vec<ReplyShape>,
    },
}

#[derive(Deserialize)]
pub(crate) struct Mapping {
    non_default_params: Object,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Deserialize)]
pub(crate) struct RequestShape {
    // LiteLLM passes the model only into error messages the prompt helper no longer builds
    #[serde(rename = "model")]
    _model: String,
    #[serde(default)]
    settings: Object,
    #[serde(default)]
    non_default_params: Object,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    messages: Vec<Message>,
}

#[derive(Deserialize)]
pub(crate) struct ReplyShape {
    body: String,
    status: u16,
    model: String,
    created: u64,
    id: String,
    finish_reason: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Options { mappings, .. } => Ok(options_value(mappings)),
            Input::Request { requests } => {
                Ok(Value::Array(requests.iter().map(request_outcome).collect()))
            }
            Input::Reply { requests } => {
                Ok(Value::Array(requests.iter().map(reply_outcome).collect()))
            }
        }
    }
}

fn options_value(mappings: &[Mapping]) -> Value {
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mappings
            .iter()
            .map(|mapping| {
                json!(map_openai_params(
                    &mapping.non_default_params,
                    mapping.optional_params.clone()
                ))
            })
            .collect::<Vec<Value>>(),
    })
}

fn request_outcome(request: &RequestShape) -> Value {
    match request_options_of(request) {
        Ok(options) => json!({"ok": true, "value": options}),
        Err(_error) => json!({"ok": false, "status": Value::Null}),
    }
}

fn request_options_of(request: &RequestShape) -> Result<Object, CohereV1ConfigError> {
    let config = CohereV1ChatConfig::with_settings(request.settings.clone())?;
    let mapped = map_openai_params(&request.non_default_params, request.optional_params.clone());
    request_options(&RequestInput {
        config: &config,
        messages: &request.messages,
        optional_params: &mapped,
    })
}

fn reply_outcome(request: &ReplyShape) -> Value {
    let input = ReplyInput {
        body: request.body.as_bytes(),
        status: request.status,
        model: &request.model,
        created: request.created,
        id: request.id.clone(),
        finish_reason: request.finish_reason.clone(),
    };
    match decode_cohere_chat_v1(input) {
        Ok(reply) => json!({"ok": true, "value": projection(&reply, &request.id)}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

fn projection(reply: &CohereV1Reply, initial_id: &str) -> Value {
    json!({
        "created": reply.created,
        "model": reply.model,
        "finish_reason": reply.finish_reason,
        "content": reply.content,
        "tool_calls": reply.tool_calls,
        "citations": reply.citations,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "completion_tokens": reply.usage.completion_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
        "id_preserved": reply.id == initial_id,
    })
}
