use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::event::{ResponseObject, SuccessEvent};
use super::PayloadError;
use super::fields::object;
use super::finish_reason::map_finish_reason;

const MODEL_CHOICE_FIELDS: [&str; 5] = [
    "finish_reason",
    "index",
    "message",
    "logprobs",
    "provider_specific_fields",
];
const TEXT_CHOICE_FIELDS: [&str; 4] = ["finish_reason", "index", "text", "logprobs"];
const MESSAGE_FIELDS: [&str; 5] = [
    "content",
    "role",
    "tool_calls",
    "function_call",
    "provider_specific_fields",
];
const DROPPED_WHEN_NULL: [&str; 6] = [
    "annotations",
    "audio",
    "images",
    "reasoning_content",
    "reasoning_items",
    "thinking_blocks",
];

pub(super) fn output(event: &SuccessEvent<'_>) -> Result<Json, PayloadError> {
    let Some(response) = &event.response else {
        return Ok(Json::List(Vec::new()));
    };
    if event.call_type == Some("embedding") || matches!(response, ResponseObject::Embedding(_)) {
        return Ok(Json::List(Vec::new()));
    }
    match response {
        ResponseObject::Model(body) => choices(body, model_choice),
        ResponseObject::TextCompletion(body) => choices(body, text_choice),
        ResponseObject::Embedding(_) | ResponseObject::Image(_) => Ok(Json::List(Vec::new())),
    }
}

fn choices(
    body: &Json,
    convert: fn(&Json) -> Result<Json, PayloadError>,
) -> Result<Json, PayloadError> {
    let fields = object(body, "response")?;
    let items = match fields.get("choices") {
        Some(Json::List(items)) => items.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            vec![Json::Object(IndexMap::new())]
        }
    };
    let converted = items.iter().map(convert).collect::<Result<Vec<_>, _>>()?;
    Ok(Json::List(converted))
}

fn model_choice(item: &Json) -> Result<Json, PayloadError> {
    let fields = object(item, "choices[]")?;
    let (finish_reason, native) = model_finish_reason(fields)?;
    let mut choice = IndexMap::new();
    choice.insert("finish_reason".to_owned(), finish_reason);
    choice.insert("index".to_owned(), model_index(fields)?);
    choice.insert("message".to_owned(), message(fields.get("message"))?);
    if let Some(logprobs) = fields
        .get("logprobs")
        .filter(|value| !matches!(value, Json::Null))
    {
        choice.insert("logprobs".to_owned(), logprobs.clone());
    }
    if let Some(provider) = choice_provider_fields(fields, native)? {
        choice.insert("provider_specific_fields".to_owned(), provider);
    }
    extras(fields, &MODEL_CHOICE_FIELDS, &mut choice);
    Ok(Json::Object(choice))
}

fn model_finish_reason(
    fields: &IndexMap<String, Json>,
) -> Result<(Json, Option<String>), PayloadError> {
    match fields.get("finish_reason") {
        None | Some(Json::Null) => Ok((Json::Text("stop".to_owned()), None)),
        Some(Json::Text(reason)) => {
            let raw = reason.as_str();
            let mapped = map_finish_reason(raw);
            let native = (mapped != raw).then(|| reason.clone());
            Ok((Json::Text(mapped.to_owned()), native))
        }
        Some(_) => Err(PayloadError::Field {
            field: "choices[].finish_reason",
            expected: "a string or null",
        }),
    }
}

fn model_index(fields: &IndexMap<String, Json>) -> Result<Json, PayloadError> {
    match fields.get("index") {
        None | Some(Json::Null) => Ok(Json::Number(Number::from(0_u8))),
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Json::Number(number.clone()))
        }
        Some(_) => Err(PayloadError::Field {
            field: "choices[].index",
            expected: "an integer",
        }),
    }
}

fn choice_provider_fields(
    fields: &IndexMap<String, Json>,
    native: Option<String>,
) -> Result<Option<Json>, PayloadError> {
    let mut provider = match fields.get("provider_specific_fields") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(provider)) => provider.clone(),
        Some(_) => {
            return Err(PayloadError::Field {
                field: "choices[].provider_specific_fields",
                expected: "a JSON object or null",
            });
        }
    };
    if let Some(reason) = native {
        provider.insert("native_finish_reason".to_owned(), Json::Text(reason));
    }
    if provider.is_empty() {
        return Ok(None);
    }
    Ok(Some(Json::Object(provider)))
}

fn text_choice(item: &Json) -> Result<Json, PayloadError> {
    let fields = object(item, "choices[]")?;
    let mut choice = IndexMap::new();
    choice.insert("finish_reason".to_owned(), text_finish_reason(fields)?);
    choice.insert(
        "index".to_owned(),
        fields
            .get("index")
            .cloned()
            .unwrap_or_else(|| Json::Number(Number::from(0_u8))),
    );
    choice.insert(
        "text".to_owned(),
        fields.get("text").cloned().unwrap_or(Json::Null),
    );
    choice.insert(
        "logprobs".to_owned(),
        fields.get("logprobs").cloned().unwrap_or(Json::Null),
    );
    extras(fields, &TEXT_CHOICE_FIELDS, &mut choice);
    Ok(Json::Object(choice))
}

fn text_finish_reason(fields: &IndexMap<String, Json>) -> Result<Json, PayloadError> {
    match fields.get("finish_reason") {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::Text(reason)) if reason.is_empty() => Ok(Json::Null),
        Some(Json::Text(reason)) => Ok(Json::Text(map_finish_reason(reason).to_owned())),
        Some(_) => Err(PayloadError::Field {
            field: "choices[].finish_reason",
            expected: "a string or null",
        }),
    }
}

fn message(value: Option<&Json>) -> Result<Json, PayloadError> {
    let empty: IndexMap<String, Json> = IndexMap::new();
    let fields = match value {
        None | Some(Json::Null) => &empty,
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            return Err(PayloadError::Field {
                field: "choices[].message",
                expected: "a JSON object",
            });
        }
    };
    let mut message = IndexMap::new();
    message.insert(
        "content".to_owned(),
        fields.get("content").cloned().unwrap_or(Json::Null),
    );
    message.insert("role".to_owned(), role(fields.get("role"))?);
    message.insert(
        "tool_calls".to_owned(),
        tool_calls(fields.get("tool_calls"))?,
    );
    message.insert(
        "function_call".to_owned(),
        object_or_null(
            fields.get("function_call"),
            "choices[].message.function_call",
        )?,
    );
    message.insert(
        "provider_specific_fields".to_owned(),
        object_or_null(
            fields.get("provider_specific_fields"),
            "choices[].message.provider_specific_fields",
        )?,
    );
    message_extras(fields, &mut message);
    Ok(Json::Object(message))
}

fn message_extras(fields: &IndexMap<String, Json>, message: &mut IndexMap<String, Json>) {
    for (key, item) in fields {
        if MESSAGE_FIELDS.contains(&key.as_str()) {
            continue;
        }
        if matches!(item, Json::Null) && DROPPED_WHEN_NULL.contains(&key.as_str()) {
            continue;
        }
        message.insert(key.clone(), item.clone());
    }
}

fn role(value: Option<&Json>) -> Result<Json, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(Json::Text("assistant".to_owned())),
        Some(Json::Text(role)) => Ok(Json::Text(role.clone())),
        Some(_) => Err(PayloadError::Field {
            field: "choices[].message.role",
            expected: "a string or null",
        }),
    }
}

fn tool_calls(value: Option<&Json>) -> Result<Json, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(Json::List(calls)) if calls.is_empty() => Ok(Json::Null),
        Some(calls @ Json::List(_)) => Ok(calls.clone()),
        Some(_) => Err(PayloadError::Field {
            field: "choices[].message.tool_calls",
            expected: "a JSON array or null",
        }),
    }
}

fn object_or_null(value: Option<&Json>, field: &'static str) -> Result<Json, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(Json::Null),
        Some(object @ Json::Object(_)) => Ok(object.clone()),
        Some(_) => Err(PayloadError::Field {
            field,
            expected: "a JSON object or null",
        }),
    }
}

fn extras(fields: &IndexMap<String, Json>, declared: &[&str], out: &mut IndexMap<String, Json>) {
    for (key, value) in fields {
        if !declared.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
}
