pub mod error;
pub mod messages;
pub mod output;
pub mod reasoning;
pub mod records;
pub mod status;

mod tests;

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage, UsageNumber};
use betterlite_openai::api::responses_objects::sdk::response::{ResponseTools, ToolChoice};
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use rust_decimal::Decimal;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_codex_tools::records::AttributeValue;
use crate::api::responses_codex_tools::tool_choices::responses_tool_choice;
use crate::api::responses_output_details::usage_details;
use crate::api::responses_tool_namespaces::records::{
    ChoiceRecord, FunctionRecord, ProviderFields, ToolCallRecord,
};
use crate::api::responses_tool_namespaces::tool_outputs::tool_outputs;
use crate::api::responses_tool_namespaces::values::has_value;
use crate::api::responses_usage::{UsageInput, normalize_usage};

use self::error::ChatOutputError;
use self::output::OutputItem;
use self::records::{
    AssembledResponse, ChatChoice, ChatCompletionResponse, ChatToolCall, RequestFields,
};

pub fn assemble_response<F>(
    _request_input: &Json,
    request: &RequestFields,
    chat: &ChatCompletionResponse,
    limits: ApiLimits,
    mut uuid: F,
) -> Result<AssembledResponse, ChatOutputError>
where
    F: FnMut() -> String,
{
    let mut items = reasoning::reasoning_items(&chat.choices, &mut uuid)?;
    items.extend(messages::message_items(&chat.choices, &mut uuid));
    let tool_records: Vec<ChoiceRecord> = chat.choices.iter().map(choice_record).collect();
    let tools = tool_outputs(&tool_records, &request.tools, limits)?;
    items.extend(tools.items.into_iter().map(OutputItem::Tool));
    let interpreter = output::interpreter_results(&chat.choices)?;
    if !interpreter.is_empty() {
        items = output::replace_function_calls(items, &interpreter);
    }
    let output_items: Vec<ResponseOutputItem> =
        items.into_iter().map(output::into_response_item).collect();
    let (response, hidden_params) = response_record(chat, request, output_items)?;
    Ok(AssembledResponse {
        response,
        hidden_params,
        cache_writes: tools.cache_writes,
    })
}

fn response_record(
    chat: &ChatCompletionResponse,
    request: &RequestFields,
    output: Vec<ResponseOutputItem>,
) -> Result<(ResponsesApiResponse, BTreeMap<String, Json>), ChatOutputError> {
    let mut response =
        serde_json::from_str::<ResponsesApiResponse>(r#"{"id":"","created_at":0,"output":[]}"#)
            .map_err(|source| ChatOutputError::Response { source })?;
    response.id.clone_from(&chat.id);
    response.created_at = chat.created;
    response.model.clone_from(&chat.model);
    response.object = Some("response".to_owned());
    response.error.clone_from(&chat.error);
    response.incomplete_details =
        status::incomplete_details(first_finish(chat), chat.incomplete_details.clone());
    response.instructions.clone_from(&chat.instructions);
    response.reasoning = None;
    response.status = Some(status::finish_status(first_finish(chat)).to_owned());
    response.text = Some(BTreeMap::new());
    response.usage = Some(response_usage(chat.usage.as_ref())?);
    response.store = None;
    response.output = output;
    overridden_fields(&mut response, chat, request)?;
    let hidden_params = chat.hidden_params.clone();
    let surfaced = hidden_params
        .get("provider_specific_fields")
        .filter(|value| has_value(value));
    if let Some(fields) = surfaced {
        response
            .extra
            .insert("provider_specific_fields".to_owned(), fields.clone());
    }
    Ok((response, hidden_params))
}

fn overridden_fields(
    response: &mut ResponsesApiResponse,
    chat: &ChatCompletionResponse,
    request: &RequestFields,
) -> Result<(), ChatOutputError> {
    response.metadata = chat
        .metadata
        .as_ref()
        .map_or_else(|| Some(BTreeMap::new()), Clone::clone);
    response.parallel_tool_calls = chat
        .parallel_tool_calls
        .as_ref()
        .map_or(Some(false), |value| *value);
    response.temperature = match &chat.temperature {
        None => Some(Decimal::new(0, 1)),
        Some(None) => None,
        Some(Some(value)) => Some(decimal_field(value, "temperature")?),
    };
    response.tool_choice = Some(tool_choice(request)?);
    response.tools = chat.tools.as_ref().map_or_else(
        || Some(ResponseTools::Definitions(Vec::new())),
        Clone::clone,
    );
    response.top_p = match &chat.top_p {
        None | Some(Json::Null) => None,
        Some(value) => Some(decimal_field(value, "top_p")?),
    };
    response.max_output_tokens = chat.max_output_tokens;
    response
        .previous_response_id
        .clone_from(&chat.previous_response_id);
    response.truncation = chat.truncation;
    response.user.clone_from(&chat.user);
    Ok(())
}

fn first_finish(chat: &ChatCompletionResponse) -> Option<&str> {
    match chat.choices.as_slice() {
        [first, ..] => first.finish_reason.as_deref(),
        [] => None,
    }
}

fn tool_choice(request: &RequestFields) -> Result<ToolChoice, ChatOutputError> {
    let normalized = responses_tool_choice(&request.tool_choice);
    let text = serde_json::to_string(&normalized).map_err(|source| ChatOutputError::Json {
        field: "tool_choice",
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| ChatOutputError::ToolChoice { source })
}

fn decimal_field(value: &Json, field: &'static str) -> Result<Decimal, ChatOutputError> {
    let Json::Number(number) = value else {
        return Err(ChatOutputError::WrongType {
            field,
            expected: "a number",
        });
    };
    Decimal::from_str(&number.to_string())
        .map_err(|source| ChatOutputError::Number { field, source })
}

fn response_usage(raw_usage: Option<&Json>) -> Result<ResponseApiUsage, ChatOutputError> {
    let Some(usage) = raw_usage else {
        return Ok(usage_details(None));
    };
    let text = serde_json::to_string(usage).map_err(|source| ChatOutputError::Json {
        field: "usage",
        source,
    })?;
    let value = serde_json::from_str(&text).map_err(|source| ChatOutputError::Json {
        field: "usage",
        source,
    })?;
    let normalized = normalize_usage(UsageInput::Mapping(&value))
        .map_err(|source| ChatOutputError::Usage { source })?;
    let mut numeric_extras = BTreeMap::new();
    if let Some(cost) = normalized.cost {
        numeric_extras.insert("cost".to_owned(), UsageNumber::Decimal(cost));
    }
    let typed = Usage {
        prompt_tokens: normalized.prompt_tokens,
        completion_tokens: normalized.completion_tokens,
        total_tokens: normalized.total_tokens,
        prompt_tokens_details: normalized.prompt_tokens_details,
        completion_tokens_details: normalized.completion_tokens_details,
        numeric_extras,
    };
    Ok(usage_details(Some(&typed)))
}

fn choice_record(choice: &ChatChoice) -> ChoiceRecord {
    let message = &choice.message;
    ChoiceRecord {
        tool_calls: message.tool_calls.iter().map(tool_call_record).collect(),
        provider_specific_fields: message
            .provider_specific_fields
            .as_ref()
            .map_or(Json::Null, |fields| Json::Object(fields.clone())),
    }
}

fn tool_call_record(call: &ChatToolCall) -> ToolCallRecord {
    let call_function = call.function.as_ref();
    ToolCallRecord {
        id: call.id.clone(),
        kind: call.kind.clone(),
        function: FunctionRecord {
            name: call_function
                .and_then(|function| function.name.clone())
                .unwrap_or_default(),
            arguments: call_function
                .and_then(|function| function.arguments.clone())
                .unwrap_or(Json::Null),
            status: call_function.and_then(|function| function.status),
            provider_specific_fields: call_function
                .and_then(|function| function.provider_specific_fields.as_ref())
                .map(provider_fields),
        },
        provider_specific_fields: call.provider_specific_fields.as_ref().map(provider_fields),
    }
}

fn provider_fields(value: &Json) -> ProviderFields {
    match value {
        Json::Object(fields) => ProviderFields::Mapping(fields.clone()),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            ProviderFields::Attributes(AttributeValue::Plain(other.clone()))
        }
    }
}
