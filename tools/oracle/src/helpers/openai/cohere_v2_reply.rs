use betterlite_openai::cohere_chat_v2::reply::citations::annotations;
use betterlite_openai::cohere_chat_v2::reply::{AssistantReply, ReplyInput, decode_cohere_chat_v2};
use betterlite_openai::cohere_chat_v2::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::{Value, json};

mod tests;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Options {
        // the pinned get_supported_openai_params takes a model and returns the same list for any
        #[serde(rename = "supported_model")]
        _supported_model: String,
        #[serde(default)]
        mappings: Vec<Mapping>,
    },
    Annotations {
        #[serde(default)]
        citations: Vec<Json>,
    },
    Reply {
        #[serde(default)]
        requests: Vec<Request>,
    },
}

#[derive(Deserialize)]
pub(crate) struct Mapping {
    non_default_params: Object,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Deserialize)]
pub(crate) struct Request {
    body: String,
    status: u16,
    model: String,
    created: u64,
    #[serde(default)]
    id: String,
    finish_reason: Option<String>,
    #[serde(default)]
    message: AssistantReply,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(match input {
            Input::Options { mappings, .. } => json!({
                "supported": SUPPORTED_OPENAI_PARAMS,
                "mapped": mappings.iter().map(|mapping| {
                    json!(map_openai_params(&mapping.non_default_params, mapping.optional_params.clone()))
                }).collect::<Vec<Value>>(),
            }),
            Input::Annotations { citations } => match annotations(citations) {
                Ok(found) => json!({"ok": true, "value": found}),
                Err(error) => json!({"ok": false, "status": error.status_code()}),
            },
            Input::Reply { requests } => Value::Array(requests.iter().map(outcome).collect()),
        })
    }
}

fn outcome(request: &Request) -> Value {
    let input = ReplyInput {
        body: request.body.as_bytes(),
        status: request.status,
        model: &request.model,
        created: request.created,
        id: request.id.clone(),
        finish_reason: request.finish_reason.clone(),
        message: request.message.clone(),
    };
    match decode_cohere_chat_v2(input) {
        Ok(reply) => json!({"ok": true, "value": {
            "created": reply.created,
            "model": reply.model,
            "finish_reason": reply.finish_reason,
            "content": reply.message.content,
            "tool_calls": reply.message.tool_calls,
            "annotations": reply.message.annotations,
            "usage": reply.usage,
            "id_preserved": reply.id == request.id,
        }}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}
