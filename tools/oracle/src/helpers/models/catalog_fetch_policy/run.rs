use std::str::FromStr as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::client::{CatalogRequest, FetchClientError};
use betterlite_models::fetch_policy::integrity::{
    IntegrityPolicy, MAX_SHRINK_RATIO, MIN_MODEL_COUNT, check_model_count_not_reduced,
    count_model_entries, validate_model_cost_map,
};
use betterlite_models::fetch_policy::outcome::{
    AttemptOutcome, FetchResult, Retryable, classify_fetch_response,
};
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fetch_policy::retry::{
    NextStep, fetch_with_retry, next_retry_step, parse_retry_after_seconds, retry_wait_seconds,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::input::{Input, ReplyInput};
use super::invalid;
use super::scripting::{RecordingSleeper, ScriptedClient, ScriptedJitter};
use crate::helpers::{HelperError, HelperResult};

fn optional_decimal(text: Option<&str>) -> Result<Option<Decimal>, Box<HelperError>> {
    text.map_or(Ok(None), |value| {
        Decimal::from_str(value)
            .map(Some)
            .map_err(|error| invalid(format!("{value}: {error}")))
    })
}

fn integrity_policy(input: &Input) -> Result<IntegrityPolicy, Box<HelperError>> {
    let min_model_count = input.min_model_count.unwrap_or(MIN_MODEL_COUNT);
    let max_shrink_ratio = match input.max_shrink_ratio.as_deref() {
        Some(text) => {
            Decimal::from_str(text).map_err(|error| invalid(format!("{text}: {error}")))?
        }
        None => MAX_SHRINK_RATIO,
    };
    Ok(IntegrityPolicy {
        min_model_count,
        max_shrink_ratio,
    })
}

fn model_keys(input: &Input) -> IndexMap<String, Json> {
    input
        .model_keys
        .iter()
        .map(|key| (key.clone(), Json::Object(IndexMap::new())))
        .collect()
}

fn validated_models(
    input: &Input,
) -> Result<(IndexMap<String, Json>, IntegrityPolicy), Box<HelperError>> {
    let models = model_keys(input);
    let policy = integrity_policy(input)?;
    Ok((models, policy))
}

pub(super) fn validate(input: &Input) -> HelperResult {
    let (models, policy) = validated_models(input)?;
    Ok(validation_value(&models, input, policy))
}

fn validation_value(
    models: &IndexMap<String, Json>,
    input: &Input,
    policy: IntegrityPolicy,
) -> Value {
    let valid = validate_model_cost_map(models, input.backup_model_count, policy).is_ok();
    let count = count_model_entries(models);
    json!({
        "valid": valid,
        "is_valid_dict": !models.is_empty(),
        "count_not_reduced": check_model_count_not_reduced(models, input.backup_model_count, policy).is_ok(),
        "count": count,
    })
}

fn sorted_models(models: &IndexMap<String, Json>) -> Vec<String> {
    let mut names = models.keys().cloned().collect::<Vec<_>>();
    names.sort_unstable();
    names
}

pub(super) fn classify(input: &Input) -> Value {
    let response = FetchResponse::new(
        input.status,
        input.headers.clone(),
        input.body.as_bytes().to_vec(),
    );
    match classify_fetch_response(response, &input.url) {
        AttemptOutcome::Reloaded(fetched) => json!({
            "kind": "reloaded",
            "revision": fetched.revision,
            "etag": fetched.etag,
            "models": sorted_models(&fetched.models),
        }),
        AttemptOutcome::Unavailable(_) => json!({"kind": "unavailable"}),
        AttemptOutcome::Retryable(retryable) => json!({
            "kind": "retryable",
            "retry_after_seconds": retryable
                .retry_after_seconds
                .map(|seconds| seconds.normalize().to_string()),
        }),
    }
}

pub(super) fn parse_retry_after(input: &Input) -> Value {
    let response = FetchResponse::new(200, input.headers.clone(), Vec::new());
    json!({
        "retry_after_seconds": parse_retry_after_seconds(&response)
            .map(|seconds| seconds.normalize().to_string()),
    })
}

fn retryable(input: &Input) -> Result<Retryable, Box<HelperError>> {
    Ok(Retryable {
        reason: "synthetic".to_owned(),
        retry_after_seconds: optional_decimal(input.retry_after.as_deref())?,
    })
}

pub(super) fn retry_wait(input: &Input) -> HelperResult {
    let retryable = retryable(input)?;
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let wait = retry_wait_seconds(&retryable, input.attempt, &mut jitter);
    Ok(json!({"wait_seconds": wait.normalize().to_string()}))
}

fn retry_step(input: &Input) -> Result<NextStep, Box<HelperError>> {
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let outcome = AttemptOutcome::Retryable(retryable(input)?);
    Ok(next_retry_step(
        outcome,
        input.attempt,
        input.max_attempts,
        &mut jitter,
    ))
}

fn step_output(step: &NextStep) -> Result<Value, Box<HelperError>> {
    match step {
        NextStep::Retry(seconds) => Ok(json!({
            "kind": "retry",
            "wait_seconds": seconds.normalize().to_string(),
        })),
        NextStep::Done(FetchResult::Unavailable(_)) => Ok(json!({"kind": "unavailable"})),
        NextStep::Done(FetchResult::Reloaded(_)) => {
            Err(invalid("a retryable outcome never reloads".to_owned()))
        }
    }
}

pub(super) fn next_retry_wait(input: &Input) -> HelperResult {
    let step = retry_step(input)?;
    let output = step_output(&step)?;
    Ok(json!(output))
}

fn scripted_replies(replies: &[ReplyInput]) -> Vec<Result<FetchResponse, FetchClientError>> {
    replies
        .iter()
        .map(|reply| {
            Ok(FetchResponse::new(
                reply.status,
                reply.headers.clone(),
                reply.body.as_bytes().to_vec(),
            ))
        })
        .collect()
}

fn fetch_output(result: FetchResult, requests: usize, sleeps: &[String]) -> Value {
    match result {
        FetchResult::Reloaded(fetched) => json!({
            "kind": "reloaded",
            "requests": requests,
            "sleeps": sleeps,
            "revision": fetched.revision,
            "etag": fetched.etag,
            "models": sorted_models(&fetched.models),
        }),
        FetchResult::Unavailable(_) => json!({
            "kind": "unavailable",
            "requests": requests,
            "sleeps": sleeps,
        }),
    }
}

pub(super) fn fetch(input: &Input) -> HelperResult {
    let client = ScriptedClient {
        replies: scripted_replies(&input.replies),
        calls: AtomicUsize::new(0),
    };
    let sleeper = RecordingSleeper {
        seconds: Mutex::new(Vec::new()),
    };
    let mut jitter = ScriptedJitter::new(&input.jitter)?;
    let request = CatalogRequest {
        url: input.url.clone(),
        timeout: Duration::from_secs(input.timeout.unwrap_or(5)),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid(error.to_string()))?;
    let result = runtime.block_on(fetch_with_retry(
        &request,
        input.max_attempts,
        &client,
        &sleeper,
        &mut jitter,
    ));
    let sleeps = sleeper
        .recorded()
        .iter()
        .map(|seconds| seconds.normalize().to_string())
        .collect::<Vec<_>>();
    Ok(fetch_output(
        result,
        client.calls.load(Ordering::SeqCst),
        &sleeps,
    ))
}
