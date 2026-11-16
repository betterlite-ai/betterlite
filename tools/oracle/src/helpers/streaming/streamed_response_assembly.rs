use betterlite_error::error::ErrorClass;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, JsonValue};
use betterlite_streaming::stream_usage_assembly::wire::Usage;
use betterlite_streaming::streamed_response_assembly::cost::StreamLogging;
use betterlite_streaming::streamed_response_assembly::records::Chunk;
use betterlite_streaming::streamed_response_assembly::response::{
    ChatChoice, ChatResponse, Response, TextResponse, TextUsage,
};
use betterlite_streaming::streamed_response_assembly::{
    AssemblyError, Request, stream_chunk_builder,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

mod fixture;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use fixture::{
    CostFixture, CounterFixture, FixtureCosts, FixtureCounter, FixtureLogging, LoggingFixture,
    failure, hooks,
};

#[path = "streamed_response_assembly/tests.rs"]
mod tests;

const ASSEMBLE: &str = "assemble";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: String,
    chunks: Option<Vec<Chunk>>,
    messages: Option<Vec<Value>>,
    count_prompt_tokens: Option<PromptCount>,
    #[serde(default)]
    counter: CounterFixture,
    #[serde(default)]
    costs: CostFixture,
    logging: Option<LoggingFixture>,
    now: i64,
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

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(error.to_string()))?;
        assemble_input(&runtime, input)
    }
}

fn assemble_input(runtime: &Runtime, input: &Input) -> HelperResult {
    if input.function != ASSEMBLE {
        return Err(failure(format!(
            "the function {} is not {ASSEMBLE}",
            input.function
        )));
    }
    let typed_messages = decode_messages(input.messages.as_deref())?;
    let fixture_counter = FixtureCounter::new(&input.counter);
    let fixture_costs = FixtureCosts::new(&input.costs);
    let fixture_logging = input
        .logging
        .as_ref()
        .map(|fixture| -> Box<dyn StreamLogging> { Box::new(FixtureLogging::new(fixture)) });
    let callback = prompt_callback(input.count_prompt_tokens.as_ref());
    let hooks = hooks()?;
    let request = Request {
        chunks: input.chunks.as_deref(),
        messages: typed_messages.as_deref(),
        count_prompt_tokens: callback.as_deref(),
        logging: fixture_logging.as_deref(),
        costs: &fixture_costs,
        counter: &fixture_counter,
        metadata: None,
        now: input.now,
    };
    match runtime.block_on(stream_chunk_builder(&request, &hooks)) {
        Ok(Some(response)) => Ok(response_view(&response)),
        Ok(None) => Ok(Value::Null),
        Err(error) => Err(assembly_error(&error)),
    }
}

fn decode_messages(messages: Option<&[Value]>) -> Result<Option<Vec<Message>>, Box<HelperError>> {
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

fn assembly_error(error: &AssemblyError) -> Box<HelperError> {
    let ErrorClass::Api(details) = error.classify() else {
        return failure(error.to_string());
    };
    Box::new(HelperError {
        class: "APIError".to_owned(),
        status: Some(details.status),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: format!("litellm.APIError: {}", details.context.message),
    })
}

fn response_view(response: &Response) -> Value {
    match response {
        Response::Chat(chat) => prune(chat_view(chat)),
        Response::Text(text) => prune(text_view(text)),
    }
}

fn chat_view(chat: &ChatResponse) -> Value {
    let choices: Vec<Value> = chat.choices.iter().map(chat_choice_view).collect();
    json!({
        "kind": "chat",
        "id": chat.id,
        "object": chat.object,
        "created": chat.created,
        "model": chat.model,
        "system_fingerprint": chat.system_fingerprint,
        "service_tier": chat.service_tier,
        "choices": choices,
        "usage": usage_view(&chat.usage),
        "hidden_params": hidden_view(&chat.hidden_params),
    })
}

fn text_view(text: &TextResponse) -> Value {
    let choices: Vec<Value> = text
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "text": choice.text,
                "logprobs": choice.logprobs,
            })
        })
        .collect();
    json!({
        "kind": "text",
        "id": text.id,
        "object": text.object,
        "created": text.created,
        "model": text.model,
        "system_fingerprint": text.system_fingerprint,
        "choices": choices,
        "usage": text_usage_view(&text.usage),
    })
}

fn chat_choice_view(choice: &ChatChoice) -> Value {
    let message = &choice.message;
    json!({
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "provider_specific_fields": choice.provider_specific_fields,
        "message": {
            "content": message.content,
            "role": message.role,
            "tool_calls": message.tool_calls,
            "function_call": message.function_call,
            "provider_specific_fields": message.provider_specific_fields,
            "reasoning_content": message.reasoning_content,
            "thinking_blocks": message.thinking_blocks,
            "annotations": message.annotations,
            "audio": message.audio,
            "images": message.images,
        },
    })
}

fn usage_view(usage: &Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "cost": usage.cost.map(|cost| format!("{cost:.12}")),
    })
}

// The response_cost prints like the usage cost so 1 and 1.0 compare equal.
fn hidden_view(hidden: &JsonObject) -> Value {
    json!({
        "response_cost": hidden
            .get("response_cost")
            .and_then(response_cost_number)
            .map(|cost| format!("{cost:.12}")),
        "provider_specific_fields": hidden.get("provider_specific_fields"),
    })
}

fn response_cost_number(cost: &JsonValue) -> Option<f64> {
    match cost {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

fn text_usage_view(usage: &TextUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    })
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
