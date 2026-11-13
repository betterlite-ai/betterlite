use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_fields::nested_filter::filter_value_from_dict;
use betterlite_prompts::schema::drop_non_python_regex_patterns;
use betterlite_prompts::schema::flatten::flatten_top_level;

use super::error::ResponseInputError;
use super::schema::decode_response_tool_schema;
use super::{ResponsesInput, ResponsesItem, ResponsesTool};

const COMBINATOR_REJECTING_FAMILIES: [&str; 6] =
    ["gpt-4", "gpt-3.5", "chatgpt-4o", "o1", "o3", "o4"];

type Sanitize = fn(&Object) -> Object;

pub(crate) fn without_cache_control(
    input: ResponsesInput,
    tools: Option<Vec<ResponsesTool>>,
) -> (ResponsesInput, Option<Vec<ResponsesTool>>) {
    let cleaned_input = match input {
        ResponsesInput::Text(text) => ResponsesInput::Text(text),
        ResponsesInput::Items(items) => ResponsesInput::Items(
            items
                .into_iter()
                .map(|item| ResponsesItem(without_cache_marker(item.0)))
                .collect(),
        ),
    };
    let cleaned_tools = tools.map(|entries| {
        entries
            .into_iter()
            .map(|tool| ResponsesTool(without_cache_marker(tool.0)))
            .collect()
    });
    (cleaned_input, cleaned_tools)
}

fn without_cache_marker(tool: Json) -> Json {
    match tool {
        Json::Object(fields) => Json::Object(filter_value_from_dict(&fields, "cache_control", 0)),
        Json::Null => Json::Null,
        Json::Bool(value) => Json::Bool(value),
        Json::Number(value) => Json::Number(value),
        Json::Text(value) => Json::Text(value),
        Json::List(value) => Json::List(value),
    }
}

pub(crate) fn tools_with_object_parameters(
    model: &str,
    provider: ProviderId,
    tools: Option<Vec<ResponsesTool>>,
) -> Result<Option<Vec<ResponsesTool>>, ResponseInputError> {
    let Some(entries) = tools else {
        return Ok(None);
    };
    let mut decoded = Vec::with_capacity(entries.len());
    for (index, tool) in entries.into_iter().enumerate() {
        decoded.push(ResponsesTool(decoded_tool_entry(
            model, provider, index, tool.0,
        )?));
    }
    Ok(Some(decoded))
}

fn decoded_tool_entry(
    model: &str,
    provider: ProviderId,
    index: usize,
    entry: Json,
) -> Result<Json, ResponseInputError> {
    let Json::Object(mut fields) = entry else {
        return Ok(entry);
    };
    let Some(parameters) = fields.shift_remove("parameters") else {
        return Ok(Json::Object(fields));
    };
    if matches!(parameters, Json::Object(_) | Json::Null) {
        fields.insert("parameters".to_owned(), parameters);
        return Ok(Json::Object(fields));
    }
    if let Json::Text(text) = &parameters
        && let Ok(object) = decode_response_tool_schema(text.as_bytes())
    {
        fields.insert("parameters".to_owned(), Json::Object(object));
        return Ok(Json::Object(fields));
    }
    Err(ResponseInputError::InvalidToolParameters {
        model: model.to_owned(),
        provider,
        index,
        found: source_type_name(&parameters),
    })
}

pub(crate) fn sanitized_tool_schemas(
    model: &str,
    provider: ProviderId,
    tools: Option<Vec<ResponsesTool>>,
    model_info: Option<&Object>,
) -> Option<Vec<ResponsesTool>> {
    let entries = tools?;
    if !matches!(provider, ProviderId::Openai | ProviderId::Azure) {
        return Some(entries);
    }
    let sanitize: Sanitize = if rejects_schema_combinators(gate_model(model, model_info)) {
        |schema| flatten_top_level(&drop_non_python_regex_patterns(schema))
    } else {
        drop_non_python_regex_patterns
    };
    Some(
        entries
            .into_iter()
            .map(|tool| ResponsesTool(sanitize_tool_value(&tool.0, sanitize)))
            .collect(),
    )
}

fn sanitize_tool_value(value: &Json, sanitize: Sanitize) -> Json {
    let Json::Object(fields) = value else {
        return value.clone();
    };
    let mut updated = fields.clone();
    if let Some(Json::Object(parameters)) = fields.get("parameters") {
        updated.insert("parameters".to_owned(), Json::Object(sanitize(parameters)));
    }
    if let Some(Json::List(nested)) = fields.get("tools") {
        updated.insert(
            "tools".to_owned(),
            Json::List(
                nested
                    .iter()
                    .map(|entry| sanitize_tool_value(entry, sanitize))
                    .collect(),
            ),
        );
    }
    Json::Object(updated)
}

fn gate_model<'source>(model: &'source str, model_info: Option<&'source Object>) -> &'source str {
    match model_info.and_then(|info| info.get("base_model")) {
        Some(Json::Text(base_model)) if !base_model.is_empty() => base_model,
        _ => model,
    }
}

fn rejects_schema_combinators(model: &str) -> bool {
    let last_component = model.rsplit('/').next().unwrap_or_default();
    let base_model = last_component
        .strip_prefix("ft:")
        .map_or(last_component, |tail| {
            tail.split(':').next().unwrap_or_default()
        });
    COMBINATOR_REJECTING_FAMILIES
        .iter()
        .any(|family| base_model.starts_with(family))
}

pub(crate) fn source_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) => {
            let literal = number.to_string();
            if literal.contains(['.', 'e', 'E']) {
                "float"
            } else {
                "int"
            }
        }
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}
