use std::borrow::Cow;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_streaming::stream_usage_assembly::details::{
    apply_grounding_request_counts, attach_cache_breakdown, capture_cache_breakdown,
};
use betterlite_streaming::stream_usage_assembly::wire::{
    CacheCreationTokenDetails, Chunk, PromptTokensDetails, ReasoningChoice,
};
use betterlite_streaming::stream_usage_assembly::{
    UsageAssemblyError, UsageRequest, UsageTokenCounter, VerboseHook, calculate_usage,
    count_reasoning_tokens,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
        #[serde(default)]
        messages: Option<Vec<Value>>,
        #[serde(default)]
        reasoning_tokens: Option<i64>,
        #[serde(default)]
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
        #[serde(default)]
        prompt_tokens_details: Option<PromptTokensDetails>,
        #[serde(default)]
        current: Option<CacheCreationTokenDetails>,
    },
    AttachCacheCreation {
        #[serde(default)]
        prompt_tokens_details: Option<PromptTokensDetails>,
        #[serde(default)]
        cache_creation_token_details: Option<CacheCreationTokenDetails>,
    },
    ApplyGrounding {
        #[serde(default)]
        prompt_tokens_details: Option<PromptTokensDetails>,
        #[serde(default)]
        web_search_requests: Option<i64>,
        #[serde(default)]
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CounterFixture {
    text: i64,
    messages: i64,
    fail_text: bool,
    fail_messages: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CounterCall {
    Text {
        text: String,
        count_response_tokens: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    Messages {
        model: String,
        messages: Option<Vec<Value>>,
    },
}

struct FixtureCounter {
    fixture: CounterFixture,
    raw_messages: Option<Vec<Value>>,
    calls: Mutex<Vec<CounterCall>>,
}

impl FixtureCounter {
    fn new(fixture: &CounterFixture, raw_messages: Option<Vec<Value>>) -> Self {
        Self {
            fixture: fixture.clone(),
            raw_messages,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: CounterCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn calls(&self) -> Value {
        let calls = self
            .calls
            .lock()
            .map(|calls| {
                calls
                    .iter()
                    .map(|call| serde_json::to_value(call).unwrap_or(Value::Null))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Value::Array(calls)
    }
}

const fn counted(failed: bool, count: i64) -> Result<i64, CountError> {
    if failed {
        return Err(CountError::MissingInput);
    }
    Ok(count)
}

#[async_trait]
impl UsageTokenCounter for FixtureCounter {
    async fn count_response_text(&self, model: &str, text: &str) -> Result<i64, CountError> {
        self.record(CounterCall::Text {
            text: text.to_owned(),
            count_response_tokens: true,
            model: Some(model.to_owned()),
        });
        counted(self.fixture.fail_text, self.fixture.text)
    }

    async fn count_text(&self, text: &str) -> Result<i64, CountError> {
        self.record(CounterCall::Text {
            text: text.to_owned(),
            count_response_tokens: true,
            model: None,
        });
        counted(self.fixture.fail_text, self.fixture.text)
    }

    async fn count_messages(
        &self,
        model: &str,
        messages: Option<&[Message]>,
    ) -> Result<i64, CountError> {
        self.record(CounterCall::Messages {
            model: model.to_owned(),
            messages: self.raw_messages.clone(),
        });
        if messages.is_none() {
            return Err(CountError::MissingInput);
        }
        counted(self.fixture.fail_messages, self.fixture.messages)
    }
}

#[derive(Default)]
struct RecordedVerbose {
    messages: Mutex<Vec<String>>,
}

impl VerboseHook for RecordedVerbose {
    fn print_verbose(&self, message: &str) {
        if let Ok(mut messages) = self.messages.lock() {
            messages.push(message.to_owned());
        }
    }
}

impl RecordedVerbose {
    fn messages(&self) -> Value {
        let messages = self
            .messages
            .lock()
            .map(|messages| messages.clone())
            .unwrap_or_default();
        json!(messages)
    }
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
        } => Ok(attach(
            prompt_tokens_details.as_ref(),
            cache_creation_token_details.as_ref(),
        )?),
        Input::ApplyGrounding {
            prompt_tokens_details,
            web_search_requests,
            google_maps_grounding_requests,
        } => Ok(grounding(
            prompt_tokens_details.as_ref(),
            *web_search_requests,
            *google_maps_grounding_requests,
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

fn attach(
    prompt_tokens_details: Option<&PromptTokensDetails>,
    cache_creation_token_details: Option<&CacheCreationTokenDetails>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let result = attach_cache_breakdown(prompt_tokens_details, cache_creation_token_details);
    let details = match result.as_deref() {
        None => Value::Null,
        Some(details) => rendered(details)?,
    };
    Ok(json!({
        "ok": true,
        "same_object": matches!(&result, None | Some(Cow::Borrowed(_))),
        "details": details,
    }))
}

fn grounding(
    prompt_tokens_details: Option<&PromptTokensDetails>,
    web_search_requests: Option<i64>,
    google_maps_grounding_requests: Option<i64>,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let result = apply_grounding_request_counts(
        prompt_tokens_details,
        web_search_requests,
        google_maps_grounding_requests,
    );
    let details = match result.as_deref() {
        None => Value::Null,
        Some(details) => rendered(details)?,
    };
    Ok(json!({
        "ok": true,
        "same_object": matches!(&result, None | Some(Cow::Borrowed(_))),
        "details": details,
    }))
}

fn calculate(
    runtime: &tokio::runtime::Runtime,
    request: &CalculateRequest<'_>,
    counter: &CounterFixture,
) -> crate::helpers::HelperResult {
    let fixture = FixtureCounter::new(counter, request.messages.map(<[Value]>::to_vec));
    let verbose = RecordedVerbose::default();
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
    let usage = match runtime.block_on(calculate_usage(&fixture, &verbose, &usage_request)) {
        Ok(usage) => usage,
        Err(error) => return Ok(error_output(&error)),
    };
    Ok(json!({
        "ok": true,
        "usage": rendered(&usage)?,
        "counter_calls": fixture.calls(),
        "verbose": verbose.messages(),
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
        UsageAssemblyError::CompletionCount { .. } => "count_failed",
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

fn failure(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
