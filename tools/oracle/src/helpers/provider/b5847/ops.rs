use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::factory::PromptFactory;
use betterlite_provider::replicate::error::ReplicateFailure;
use betterlite_provider::replicate::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_provider::replicate::poll::ReplicatePolling;
use betterlite_provider::replicate::reply::{ReplicateUsage, ReplyInput, transform_response};
use betterlite_provider::replicate::request::{
    ReplicateBody, RequestContext, RequestInput, get_complete_url, transform_request,
    validate_environment,
};
use chrono::DateTime;
use serde_json::{Value, json};

use super::script::{
    FIXED_NOW, SilentLogger, StandInCounter, factory, header_map, header_map_output, helper_error,
    runtime,
};
use super::{HeaderRequest, MapCase, ReplyCase, RequestCase};
use crate::helpers::{HelperError, HelperResult};

pub(super) const fn polling_for(
    retries: i32,
    retry_delay_seconds: f64,
    polling_delay_seconds: f64,
) -> ReplicatePolling {
    ReplicatePolling {
        retries,
        retry_delay_seconds,
        polling_delay_seconds,
        model_name_with_id_length: 64,
    }
}

pub(super) fn failure_output(failure: &ReplicateFailure) -> Value {
    failure.api().map_or_else(
        || {
            json!({
                "ok": false,
                "status": null,
                "message": failure.to_string(),
            })
        },
        |error| {
            json!({
                "ok": false,
                "status": error.status_code,
                "message": error.message,
            })
        },
    )
}

pub(super) fn usage_value(usage: &ReplicateUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
}

#[must_use]
pub(super) fn map_params(cases: &[MapCase]) -> Value {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut optional_params = case.optional_params.clone();
        map_openai_params(&case.non_default, &mut optional_params, &case.model);
        results.push(json!({"options": optional_params}));
    }
    json!({"cases": results, "supported": SUPPORTED_OPENAI_PARAMS})
}

#[must_use]
pub(super) fn urls(models: &[String]) -> Value {
    let mut found = Vec::with_capacity(models.len());
    for model in models {
        found.push(get_complete_url(model));
    }
    json!({"urls": found})
}

pub(super) fn headers(requests: &[HeaderRequest]) -> HelperResult {
    let mut found = Vec::with_capacity(requests.len());
    for request in requests {
        let incoming = header_map(&request.headers)?;
        let headers = validate_environment(&incoming, request.api_key.as_deref())
            .map_err(|failure| helper_error("ReplicateError", failure))?;
        found.push(Value::Object(header_map_output(&headers)?));
    }
    Ok(json!({"headers": found}))
}

fn request_outcome(
    runtime: &tokio::runtime::Runtime,
    prompts: &PromptFactory,
    case: &RequestCase,
) -> Value {
    let mut messages = case.messages.clone();
    let mut optional_params = case.optional_params.clone();
    let outcome = runtime.block_on(transform_request(
        RequestInput {
            model: &case.model,
            messages: &mut messages,
            optional_params: &mut optional_params,
            defaults: &case.defaults,
            custom_prompts: &case.custom_prompt_dict,
        },
        &RequestContext {
            prompts,
            now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
        },
    ));
    match outcome {
        Ok(body) => json!({
            "ok": true,
            "body": body,
            "messages": messages,
            "options": optional_params,
        }),
        Err(failure) => failure_output(&failure),
    }
}

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let runtime = runtime()?;
    let prompts = factory();
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(request_outcome(&runtime, &prompts, case));
    }
    Ok(json!({"requests": results}))
}

fn reply_outcome(
    runtime: &tokio::runtime::Runtime,
    counter: &StandInCounter,
    logger: &SilentLogger,
    case: &ReplyCase,
) -> Result<Value, Box<HelperError>> {
    let headers = header_map(&case.headers)?;
    let request_data = ReplicateBody {
        input: Object::new(),
        version: None,
    };
    let input = ReplyInput {
        model: &case.model,
        body: case.body.as_bytes(),
        headers: &headers,
        messages: &case.messages,
        request_data: &request_data,
        api_key: None,
    };
    match runtime.block_on(transform_response(&input, counter, logger)) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "model": reply.model,
            "text": reply.text,
            "usage": usage_value(&reply.usage),
        })),
        Err(failure) => Ok(failure_output(&failure)),
    }
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let counter = StandInCounter;
    let logger = SilentLogger;
    let replies: Vec<Value> = cases
        .iter()
        .map(|case| reply_outcome(&runtime, &counter, &logger, case))
        .collect::<Result<_, _>>()?;
    Ok(json!({"replies": replies}))
}
