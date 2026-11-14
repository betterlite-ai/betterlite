use std::collections::BTreeMap;

use betterlite_anthropic::completion::error::{CompletionError, anthropic_text_error};
use betterlite_anthropic::completion::headers::validate_environment;
use betterlite_anthropic::completion::model;
use betterlite_anthropic::completion::options::{
    OptionDefaults, SUPPORTED_OPENAI_PARAMS, map_openai_params,
};
use betterlite_anthropic::completion::request::transform_request;
use betterlite_anthropic::completion::response::{ResponseInput, transform_response};
use betterlite_anthropic::completion::stream::chunk_parser;
use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::fixture::{Fixture, blank_response, tiktoken};
use super::input::{AuthCase, Constructor, ErrorCase, OptionCase, PromptCase, ReplyCase};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

// the constructor's DEFAULT_MAX_TOKENS
const DEFAULT_MAX_TOKENS: i64 = 4096;
// a reply made now carries a Unix time far past this, and ours carries the fixed clock
const CREATED_FLOOR: u64 = 1_000_000_000;

pub(super) fn select(models: &[String]) -> Value {
    let selected: Vec<bool> = models
        .iter()
        .map(|name| model::select(name).is_some())
        .collect();
    json!({ "selected": selected })
}

// a registered template stays registered, so a case sees every custom prompt set up to itself
pub(super) fn prompts(cases: &[PromptCase]) -> HelperResult {
    let fixture = Fixture::new()?;
    let mut custom_prompt_dict = BTreeMap::new();
    let mut texts = Vec::new();
    for case in cases {
        if let Some(entry) = &case.custom_prompt {
            custom_prompt_dict.insert(case.model.clone(), entry.clone());
        }
        let source = fixture.source(&custom_prompt_dict);
        let body = fixture
            .runtime
            .block_on(transform_request(
                &source,
                &case.model,
                &case.messages,
                IndexMap::new(),
                &OptionDefaults::default(),
            ))
            .or_value_error()?;
        let Some(Json::Text(prompt)) = body.get("prompt") else {
            return Err("the request body has no text prompt").or_value_error();
        };
        texts.push(prompt.clone());
    }
    Ok(json!({ "prompts": texts }))
}

pub(super) fn auth(cases: &[AuthCase]) -> HelperResult {
    let results = cases.iter().map(auth_case).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "results": results }))
}

fn auth_case(case: &AuthCase) -> Result<Value, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in &case.headers {
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).or_value_error()?,
            HeaderValue::from_str(value).or_value_error()?,
        );
    }
    let api_key = case.api_key.clone().map(SecretString::from);
    match validate_environment(headers, api_key.as_ref()) {
        Ok(accepted) => {
            let mut sent = BTreeMap::new();
            for (name, value) in &accepted {
                sent.insert(name.as_str(), value.to_str().or_value_error()?);
            }
            Ok(json!({ "ok": true, "headers": sent }))
        }
        Err(error @ CompletionError::MissingApiKey) => {
            Ok(json!({ "ok": false, "message": error.to_string() }))
        }
        Err(error) => Err(error).or_value_error(),
    }
}

pub(super) fn options(constructor: &Constructor, cases: &[OptionCase]) -> HelperResult {
    let configured = OptionValues {
        max_tokens_to_sample: constructor.max_tokens_to_sample,
        stop_sequences: constructor.stop_sequences.clone(),
        top_k: constructor.top_k,
        metadata: constructor.metadata.clone(),
        ..OptionValues::default()
    };
    let defaults =
        OptionDefaults::resolve(&configured, &OptionValues::default(), DEFAULT_MAX_TOKENS);
    let fixture = Fixture::new()?;
    let custom_prompt_dict = BTreeMap::new();
    let source = fixture.source(&custom_prompt_dict);
    let mut answers = Vec::new();
    for case in cases {
        let mapped =
            map_openai_params(&case.non_default, case.existing.clone(), false).or_value_error()?;
        let body = fixture
            .runtime
            .block_on(transform_request(
                &source,
                &case.model,
                &case.messages,
                mapped.clone(),
                &defaults,
            ))
            .or_value_error()?;
        answers.push(json!({ "mapped": mapped, "body": body }));
    }
    Ok(json!({ "supported": SUPPORTED_OPENAI_PARAMS, "cases": answers }))
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let fixture = Fixture::new()?;
    let tokenizer = tiktoken()?;
    let custom_prompt_dict = BTreeMap::new();
    let source = fixture.source(&custom_prompt_dict);
    let mut answers = Vec::new();
    for case in cases {
        let input = ResponseInput {
            model: &case.model,
            messages: &case.messages,
            status_code: case.status,
            body: case.body.as_bytes(),
        };
        let response = blank_response(&case.initial_content)?;
        let done = fixture
            .runtime
            .block_on(transform_response(&source, &input, response, &tokenizer));
        answers.push(match done {
            Ok(reply) => {
                let choice = reply
                    .choices
                    .first()
                    .ok_or("the reply has no choices")
                    .or_value_error()?;
                json!({
                    "ok": true,
                    "content": choice.message.content,
                    "finish_reason": choice.finish_reason,
                    "id": reply.id,
                    "model": reply.model,
                    "created_is_current": reply.created > CREATED_FLOOR,
                    "usage": [
                        reply.usage.prompt_tokens,
                        reply.usage.completion_tokens,
                        reply.usage.total_tokens,
                    ],
                })
            }
            Err(CompletionError::Upstream(error)) => {
                json!({ "ok": false, "status": error.status_code, "message": error.message })
            }
            Err(error) => return Err(error).or_value_error(),
        });
    }
    Ok(json!({ "replies": answers }))
}

pub(super) fn chunks(input: &[IndexMap<String, Json>]) -> HelperResult {
    let mut parsed = Vec::new();
    for chunk in input {
        let found = chunk_parser(chunk).or_value_error()?;
        parsed.push(json!({
            "text": found.text,
            "is_finished": found.is_finished,
            "finish_reason": found.finish_reason,
            "index": found.index,
        }));
    }
    Ok(json!({ "chunks": parsed }))
}

pub(super) fn errors(cases: &[ErrorCase]) -> Value {
    let views: Vec<Value> = cases
        .iter()
        .map(|case| {
            let error = anthropic_text_error(case.status_code, case.message.clone());
            json!({
                "status_code": error.status_code,
                "message": error.message,
                "method": error.request.method,
                "url": error.request.url,
                "response_status": error.response.status_code,
                "headers_kept": error.headers.is_some(),
            })
        })
        .collect();
    json!({ "errors": views })
}
