use std::borrow::Cow;

use betterlite_streaming::stream_usage_assembly::details::{
    apply_grounding_request_counts, attach_cache_breakdown, capture_cache_breakdown,
};
use betterlite_streaming::stream_usage_assembly::wire::{
    CacheCreationTokenDetails, Chunk, PromptTokensDetails, ReasoningChoice,
};
use betterlite_streaming::stream_usage_assembly::{
    UsageAssemblyError, UsageRequest, calculate_usage, count_reasoning_tokens,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod fixture;

use fixture::{CounterFixture, FixtureCounter, failure, hooks};

#[path = "stream_usage_assembly/tests.rs"]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Batch {
        cases: Vec<Self>,
    },
    CalculateUsage {
        chunks: Vec<Chunk>,
        model: String,
        completion_output: String,
        messages: Option<Vec<Value>>,
        reasoning_tokens: Option<i64>,
        count_prompt_tokens: Option<PromptCount>,
        #[serde(default)]
        counter: CounterFixture,
    },
    CountReasoningTokens {
        choices: Vec<ReasoningChoice>,
        #[serde(default)]
        counter: CounterFixture,
    },
    CaptureCacheCreation {
        prompt_tokens_details: Option<PromptTokensDetails>,
        current: Option<CacheCreationTokenDetails>,
    },
    AttachCacheCreation {
        prompt_tokens_details: Option<PromptTokensDetails>,
        cache_creation_token_details: Option<CacheCreationTokenDetails>,
    },
    ApplyGrounding {
        prompt_tokens_details: Option<PromptTokensDetails>,
        web_search_requests: Option<i64>,
        google_maps_grounding_requests: Option<i64>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PromptCount {
    Value(i64),
    Failure(FailureMode),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureMode {
    Raise,
}

struct CalculateRequest<'request> {
    chunks: &'request [Chunk],
    model: &'request str,
    completion_output: &'request str,
    messages: Option<&'request [Value]>,
    reasoning_tokens: Option<i64>,
    count_prompt_tokens: Option<&'request PromptCount>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        run_input(&runtime, input)
    }
}

fn run_input(runtime: &tokio::runtime::Runtime, input: &Input) -> crate::helpers::HelperResult {
    match input {
        Input::Batch { cases } => run_batch(runtime, cases),
        Input::CalculateUsage { .. } => calculate_input(runtime, input),
        Input::CountReasoningTokens { choices, counter } => {
            Ok(reasoning(runtime, choices, counter))
        }
        Input::CaptureCacheCreation {
            prompt_tokens_details,
            current,
        } => {
            let captured = capture_cache_breakdown(prompt_tokens_details.as_ref(), current.clone());
            let details = match captured.as_ref() {
                None => Value::Null,
                Some(breakdown) => rendered(breakdown)?,
            };
            Ok(json!({"ok": true, "details": details}))
        }
        Input::AttachCacheCreation {
            prompt_tokens_details,
            cache_creation_token_details,
        } => Ok(detail_update(
            attach_cache_breakdown(
                prompt_tokens_details.as_ref(),
                cache_creation_token_details.as_ref(),
            )
            .as_ref(),
        )?),
        Input::ApplyGrounding {
            prompt_tokens_details,
            web_search_requests,
            google_maps_grounding_requests,
        } => Ok(detail_update(
            apply_grounding_request_counts(
                prompt_tokens_details.as_ref(),
                *web_search_requests,
                *google_maps_grounding_requests,
            )
            .as_ref(),
        )?),
    }
}

fn calculate_input(
    runtime: &tokio::runtime::Runtime,
    input: &Input,
) -> crate::helpers::HelperResult {
    let Input::CalculateUsage {
        chunks,
        model,
        completion_output,
        messages,
        reasoning_tokens,
        count_prompt_tokens,
        counter,
    } = input
    else {
        return Err(failure(
            "the input is not a calculate_usage call".to_owned(),
        ));
    };
    calculate(
        runtime,
        &CalculateRequest {
            chunks,
            model,
            completion_output,
            messages: messages.as_deref(),
            reasoning_tokens: *reasoning_tokens,
            count_prompt_tokens: count_prompt_tokens.as_ref(),
        },
        counter,
    )
}

fn run_batch(runtime: &tokio::runtime::Runtime, cases: &[Input]) -> crate::helpers::HelperResult {
    let results = cases
        .iter()
        .map(|case| run_input(runtime, case))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(results))
}

fn reasoning(
    runtime: &tokio::runtime::Runtime,
    choices: &[ReasoningChoice],
    counter: &CounterFixture,
) -> Value {
    let fixture = FixtureCounter::new(counter, None);
    match runtime.block_on(count_reasoning_tokens(&fixture, choices)) {
        Ok(tokens) => json!({
            "ok": true,
            "reasoning_tokens": tokens,
            "counter_calls": fixture.calls(),
        }),
        Err(error) => error_output(&error),
    }
}

fn detail_update(
    result: Option<&Cow<'_, PromptTokensDetails>>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let same_object = result.is_none_or(|cow| matches!(cow, Cow::Borrowed(_)));
    let details = match result {
        None => Value::Null,
        Some(details) => rendered(details.as_ref())?,
    };
    Ok(json!({"ok": true, "same_object": same_object, "details": details}))
}

fn calculate(
    runtime: &tokio::runtime::Runtime,
    request: &CalculateRequest<'_>,
    counter: &CounterFixture,
) -> crate::helpers::HelperResult {
    let fixture = FixtureCounter::new(counter, request.messages.map(<[Value]>::to_vec));
    let hooks = hooks()?;
    let typed_messages = decode_messages(request.messages)?;
    let callback = prompt_callback(request.count_prompt_tokens);
    let usage_request = UsageRequest {
        chunks: request.chunks,
        model: request.model,
        completion_output: request.completion_output,
        messages: typed_messages.as_deref(),
        reasoning_tokens: request.reasoning_tokens,
        count_prompt_tokens: callback.as_deref(),
    };
    let usage = match runtime.block_on(calculate_usage(&fixture, &hooks, &usage_request)) {
        Ok(usage) => usage,
        Err(error) => return Ok(error_output(&error)),
    };
    Ok(json!({
        "ok": true,
        "usage": rendered(&usage)?,
        "counter_calls": fixture.calls(),
        "verbose": hooks.logger.messages(),
    }))
}

fn decode_messages(
    messages: Option<&[Value]>,
) -> Result<Option<Vec<Message>>, Box<crate::helpers::HelperError>> {
    let Some(raw) = messages else {
        return Ok(None);
    };
    let decoded = serde_json::from_value(Value::Array(raw.to_vec()))
        .map_err(|error| failure(error.to_string()))?;
    Ok(Some(decoded))
}

fn prompt_callback(
    prompt: Option<&PromptCount>,
) -> Option<Box<dyn Fn() -> Result<i64, CountError> + Send + Sync>> {
    match prompt {
        None => None,
        Some(PromptCount::Value(value)) => {
            let counted = *value;
            Some(Box::new(move || Ok(counted)))
        }
        Some(PromptCount::Failure(_)) => Some(Box::new(|| Err(CountError::MissingInput))),
    }
}

fn error_output(error: &UsageAssemblyError) -> Value {
    let kind = match error {
        UsageAssemblyError::CompletionCount(..) => "count_failed",
        UsageAssemblyError::ServerToolUse { .. } => "invalid_server_tool_use",
        UsageAssemblyError::Overflow => "overflow",
    };
    json!({"ok": false, "error_type": kind})
}

fn rendered<T>(value: &T) -> Result<Value, Box<crate::helpers::HelperError>>
where
    T: Serialize,
{
    serde_json::to_value(value)
        .map(prune)
        .map_err(|error| failure(error.to_string()))
}

fn prune(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_key, item)| !item.is_null())
                .map(|(key, item)| (key, prune(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(prune).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}
