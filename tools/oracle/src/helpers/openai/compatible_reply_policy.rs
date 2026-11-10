use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::compatible_embedding::request_body;
use betterlite_openai::compatible_wire::{
    EndpointType, ReplyOptions, decode_compatible_chat_reply, validate_environment,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    validate: Vec<Validate>,
    #[serde(default)]
    transform: Vec<Transform>,
    #[serde(default)]
    options: Vec<EmbeddingOptions>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Validate {
    api_key: Option<String>,
    api_base: Option<String>,
    endpoint: EndpointType,
    headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    custom_endpoint: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transform {
    reply: Value,
    #[serde(default)]
    json_mode: bool,
    provider: Option<String>,
    base_model: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingOptions {
    model: String,
    input: Json,
    options: IndexMap<String, Json>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let validated = input.validate.iter().map(validate).collect::<Vec<_>>();
        let transformed = input
            .transform
            .iter()
            .map(transform)
            .collect::<Result<Vec<_>, _>>()?;
        let bodies = input
            .options
            .iter()
            .map(|call| request_body(call.model.clone(), &call.input, call.options.clone()));
        Ok(
            json!({ "validate": validated, "transform": transformed, "options": bodies.collect::<Vec<_>>() }),
        )
    }
}

fn validate(call: &Validate) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let checked = validate_environment(
        key.as_ref(),
        call.api_base.as_deref(),
        call.endpoint,
        call.headers.clone(),
        call.custom_endpoint,
    );
    match checked {
        Ok((url, headers)) => json!({ "url": url, "headers": headers }),
        Err(error) => json!({ "status": error.status_code, "message": error.message }),
    }
}

fn transform(call: &Transform) -> Result<Value, Box<HelperError>> {
    // a helper reads no wall clock or random source, and the projection never shows either filled value
    let options = ReplyOptions {
        json_mode: call.json_mode,
        provider: call.provider.as_deref(),
        base_model: call.base_model.as_deref(),
        identifier: Uuid::nil,
        clock: i64::default,
    };
    let reply = decode_compatible_chat_reply(call.reply.to_string().as_bytes(), &options).map_err(
        |error| {
            Box::new(HelperError {
                class: String::from("InvalidReply"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        },
    )?;
    let usage = &reply.response.usage;
    let choices = reply.response.choices.iter().map(|choice| {
        let calls = choice.message.tool_calls.as_ref();
        json!({ "content": choice.message.content, "tool_calls": calls.map(Vec::len) })
    });
    Ok(json!({
        "model": reply.response.model,
        "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
        "choices": choices.collect::<Vec<_>>(),
        "hidden_model": reply.hidden_model,
    }))
}

mod tests;
