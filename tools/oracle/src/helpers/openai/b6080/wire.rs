use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::openrouter_chat::ChatRequest;
use betterlite_openai::openrouter_embedding::transform_embedding_response;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use crate::helpers::{HelperResult, OrHelperError as _};

use super::chat::{config, prompt_object};
use super::input::{ReplyCase, RequestCase};

const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let config = config();
    let litellm_params = Object::new();
    let headers = Headers::new();
    let mut bodies = Vec::with_capacity(cases.len());
    for case in cases {
        let optional_params = prompt_object(&case.optional_params);
        let body = config
            .transform_request(&ChatRequest {
                model: &case.model,
                messages: &case.messages,
                optional_params: &optional_params,
                litellm_params: &litellm_params,
                headers: &headers,
            })
            .or_value_error()?;
        bodies.push(serde_json::to_value(&body).or_value_error()?);
    }
    Ok(Value::Array(bodies))
}

pub(super) fn responses(cases: &[ReplyCase]) -> HelperResult {
    let config = config();
    let mut costs = Vec::with_capacity(cases.len());
    for case in cases {
        let reply = config
            .transform_response(case.body.as_bytes(), AdditionalHeaders::new())
            .or_value_error()?;
        let cost = match reply.additional_headers.get(COST_HEADER) {
            Some(cost) => serde_json::to_value(cost).or_value_error()?,
            None => Value::Null,
        };
        costs.push(json!({"cost": cost}));
    }
    Ok(Value::Array(costs))
}

pub(super) fn embedding_replies(cases: &[ReplyCase]) -> HelperResult {
    let mut replies = Vec::with_capacity(cases.len());
    for case in cases {
        let mut logged = Vec::new();
        let reply = transform_embedding_response(case.body.as_bytes(), &mut |text: &str| {
            logged.push(text.to_owned());
        })
        .or_value_error()?;
        let data: Vec<Value> = reply
            .data
            .iter()
            .map(|record| {
                json!({
                    "object": record.object,
                    "embedding": record.embedding,
                    "index": record.index,
                })
            })
            .collect();
        replies.push(json!({
            "response": {
                "object": reply.object,
                "data": data,
                "model": reply.model,
                "usage": {
                    "prompt_tokens": reply.usage.prompt_tokens,
                    "total_tokens": reply.usage.total_tokens,
                },
            },
            "logged": logged,
        }));
    }
    Ok(Value::Array(replies))
}
