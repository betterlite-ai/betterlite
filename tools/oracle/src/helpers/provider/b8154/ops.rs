use std::collections::BTreeMap;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_provider::triton::config::{
    SUPPORTED_OPENAI_PARAMS, TritonEndpoint, complete_url, map_openai_params, validate_environment,
};
use betterlite_provider::triton::embedding::{
    EmbeddingConfig as _, SUPPORTED_PARAMS as EMBEDDING_PARAMS, TextTokenCounter,
    TritonEmbeddingConfig, split_embedding_by_shape, transform_embedding_request,
    transform_embedding_response,
};
use betterlite_provider::triton::error::TritonError;
use betterlite_provider::triton::generate::{
    TritonGenerateConfig, max_tokens_default, transform_response as generate_response,
};
use betterlite_provider::triton::infer::{
    transform_request as infer_request, transform_response as infer_response,
};
use betterlite_provider::triton::request::TransformInput;
use betterlite_provider::triton::stream::decode_stream_line;
use chrono::DateTime;
use http::HeaderMap;
use serde_json::Value;
use serde_json::json;

use super::script::{
    EmbeddingReplyCase, MapCase, ReplyCase, RequestCase, SplitCase, UrlCase, factory, runtime,
};
use crate::helpers::{HelperError, HelperResult};

struct WordsCounter;

impl TextTokenCounter for WordsCounter {
    fn count_text(&self, _model: &str, text: &str) -> Option<usize> {
        Some(text.split_whitespace().count())
    }
}

fn transform_input(case: &RequestCase) -> TransformInput<'_> {
    TransformInput {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.options,
    }
}

pub(super) fn llm_type(bases: &[String], urls: &[UrlCase], mapping: &[MapCase]) -> Value {
    let found: Vec<Value> = bases
        .iter()
        .map(|base| match TritonEndpoint::from_api_base(base) {
            Ok(TritonEndpoint::Generate) => json!({"ok": true, "llm_type": "generate"}),
            Ok(TritonEndpoint::Infer) => json!({"ok": true, "llm_type": "infer"}),
            Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
        })
        .collect();
    let found_urls: Vec<Value> = urls
        .iter()
        .map(
            |case| match complete_url(case.api_base.as_deref(), case.stream) {
                Ok(url) => json!({"ok": true, "url": url}),
                Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
            },
        )
        .collect();
    let mapped: Vec<Value> = mapping
        .iter()
        .map(|case| {
            let mut optional = case.optional.clone();
            map_openai_params(&case.non_default, &mut optional);
            json!({"options": optional})
        })
        .collect();
    json!({
        "bases": found,
        "urls": found_urls,
        "supported": SUPPORTED_OPENAI_PARAMS,
        "mapped": mapped,
        "validated": validated_headers(&validate_environment()),
    })
}

fn validated_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

pub(super) fn generate_requests(
    env: &BTreeMap<String, String>,
    requests: &[RequestCase],
) -> HelperResult {
    let prompts = factory();
    let default = max_tokens_default(env).map_err(|error| {
        Box::new(HelperError {
            class: String::from("TritonError"),
            status: error.status(),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let config = TritonGenerateConfig {
        prompts: &prompts,
        now: DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default(),
        max_tokens_default: default,
    };
    let runtime = runtime()?;
    let bodies: Vec<Value> = requests
        .iter()
        .map(
            |case| match runtime.block_on(config.transform_request(&transform_input(case))) {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
            },
        )
        .collect();
    Ok(json!({"bodies": bodies}))
}

pub(super) fn infer_requests(requests: &[RequestCase]) -> Value {
    let bodies: Vec<Value> = requests
        .iter()
        .map(|case| match infer_request(&transform_input(case)) {
            Ok(body) => json!({"ok": true, "body": body}),
            Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
        })
        .collect();
    json!({"bodies": bodies})
}

fn decoded_replies<Decode>(cases: &[ReplyCase], decode: Decode) -> Value
where
    Decode: Fn(u16, &[u8]) -> Result<Option<String>, Box<TritonError>>,
{
    let found: Vec<Value> = cases
        .iter()
        .map(|case| match decode(case.status, case.body.as_bytes()) {
            Ok(text) => json!({"ok": true, "text": text}),
            Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
        })
        .collect();
    json!({"replies": found})
}

pub(super) fn generate_replies(cases: &[ReplyCase]) -> Value {
    decoded_replies(cases, |status, body| {
        generate_response(status, body)
            .map(|reply| reply.text)
            .map_err(Box::new)
    })
}

pub(super) fn infer_replies(cases: &[ReplyCase]) -> Value {
    decoded_replies(cases, |status, body| {
        infer_response(status, body)
            .map(|reply| reply.text)
            .map_err(Box::new)
    })
}

pub(super) fn stream_lines(lines: &[String]) -> Value {
    let chunks: Vec<Value> = lines
        .iter()
        .map(|line| match decode_stream_line(line) {
            Ok(chunk) => json!({
                "ok": true,
                "text": chunk.text,
                "is_finished": chunk.is_finished,
                "finish_reason": chunk.finish_reason,
                "index": chunk.index,
            }),
            Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
        })
        .collect();
    json!({"chunks": chunks})
}

pub(super) fn embedding(
    inputs: &[EmbeddingInput],
    replies: &[EmbeddingReplyCase],
    split: &[SplitCase],
) -> Value {
    let requests: Vec<Value> = inputs
        .iter()
        .map(|input| json!({"ok": true, "body": transform_embedding_request(input.clone())}))
        .collect();
    json!({
        "supported": EMBEDDING_PARAMS,
        "headers": validated_headers(&TritonEmbeddingConfig.validate_environment()),
        "requests": requests,
        "replies": embedding_replies(replies),
        "split": embedding_splits(split),
    })
}

fn embedding_replies(replies: &[EmbeddingReplyCase]) -> Vec<Value> {
    replies
        .iter()
        .map(|case| {
            let request = transform_embedding_request(case.input.clone());
            let decoded = transform_embedding_response(
                case.status,
                case.body.as_bytes(),
                &case.model,
                &request,
                &WordsCounter,
            );
            match decoded {
                Ok(reply) => json!({
                    "ok": true,
                    "model": reply.model,
                    "data": reply.data,
                    "usage": {
                        "prompt_tokens": reply.usage.prompt_tokens,
                        "completion_tokens": reply.usage.completion_tokens,
                        "total_tokens": reply.usage.total_tokens,
                    },
                }),
                Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
            }
        })
        .collect()
}

fn embedding_splits(split: &[SplitCase]) -> Vec<Value> {
    let mut results = Vec::with_capacity(split.len());
    for case in split {
        let found = match split_embedding_by_shape(&case.data, &case.shape) {
            Ok(chunks) => json!({"ok": true, "chunks": chunks}),
            Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
        };
        results.push(found);
    }
    results
}
