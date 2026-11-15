use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::openrouter_chat::ChatRequest;
use betterlite_openai::openrouter_embedding::{EmbeddingDecodeError, transform_embedding_response};
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::chat::config;
use super::input::{ReplyCase, RequestCase};

const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let config = config();
    let bodies = cases
        .iter()
        .map(|case| {
            config
                .transform_request(&ChatRequest {
                    model: &case.model,
                    messages: &case.messages,
                    optional_params: &case.optional_params,
                    litellm_params: &Object::new(),
                    headers: &Headers::new(),
                })
                .or_value_error()
        })
        .collect::<Result<Vec<Object>, Box<HelperError>>>()?;
    serde_json::to_value(&bodies).or_value_error()
}

fn reply_cost(case: &ReplyCase) -> HelperResult {
    let config = config();
    let reply = config
        .transform_response(case.body.as_bytes(), AdditionalHeaders::new())
        .or_value_error()?;
    let cost = reply.additional_headers.get(COST_HEADER);
    Ok(json!({"cost": cost}))
}

pub(super) fn responses(cases: &[ReplyCase]) -> HelperResult {
    let mut costs = Vec::with_capacity(cases.len());
    for case in cases {
        costs.push(reply_cost(case)?);
    }
    Ok(Value::Array(costs))
}

fn embedding_body(reply: &EmbeddingResponse, logged: &[String]) -> Value {
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
    json!({
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
    })
}

fn embedding_reply(case: &ReplyCase) -> HelperResult {
    let mut logged = Vec::new();
    let decoded = transform_embedding_response(case.body.as_bytes(), &mut |text: &str| {
        logged.push(text.to_owned());
    });
    let reply = match decoded {
        Ok(reply) => reply,
        Err(EmbeddingDecodeError::Provider {
            message,
            status_code,
        }) => {
            return Err(Box::new(HelperError {
                class: String::from("ProviderError"),
                status: Some(status_code),
                error_type: None,
                param: None,
                code: None,
                message,
            }));
        }
        Err(other) => return Err(other).or_value_error(),
    };
    Ok(embedding_body(&reply, &logged))
}

pub(super) fn embedding_replies(cases: &[ReplyCase]) -> HelperResult {
    let mut replies = Vec::with_capacity(cases.len());
    for case in cases {
        replies.push(embedding_reply(case)?);
    }
    Ok(Value::Array(replies))
}
