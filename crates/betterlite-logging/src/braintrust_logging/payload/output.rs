use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::event::{ResponseObject, SuccessEvent};
use super::PayloadError;
use super::finish_reason::map_finish_reason;
use super::object;

const MODEL_FIELDS: [&str; 5] = [
    "finish_reason",
    "index",
    "message",
    "logprobs",
    "provider_specific_fields",
];
const TEXT_FIELDS: [&str; 4] = ["finish_reason", "index", "text", "logprobs"];
const DROPPED_WHEN_NULL: [&str; 6] = [
    "annotations",
    "audio",
    "images",
    "reasoning_content",
    "reasoning_items",
    "thinking_blocks",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChoiceKind {
    Model,
    Text,
}

pub(super) fn output(event: &SuccessEvent<'_>) -> Result<Json, PayloadError> {
    let Some(response) = &event.response else {
        return Ok(Json::List(Vec::new()));
    };
    if event.call_type == Some("embedding") || matches!(response, ResponseObject::Embedding(_)) {
        return Ok(Json::List(Vec::new()));
    }
    match response {
        ResponseObject::Model(body) => choices(body, ChoiceKind::Model),
        ResponseObject::TextCompletion(body) => choices(body, ChoiceKind::Text),
        ResponseObject::Embedding(_) | ResponseObject::Image(_) => Ok(Json::List(Vec::new())),
    }
}

fn choices(body: &Json, kind: ChoiceKind) -> Result<Json, PayloadError> {
    let fields = object(body, "response")?;
    let items = match fields.get("choices") {
        Some(Json::List(items)) => items.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            vec![Json::Object(IndexMap::new())]
        }
    };
    let converted = items
        .iter()
        .map(|item| choice(item, kind))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::List(converted))
}

fn choice(item: &Json, kind: ChoiceKind) -> Result<Json, PayloadError> {
    let fields = object(item, "choices[]")?;
    let (finish, native) = finish_reason(fields.get("finish_reason"), kind)?;
    let mut choice = IndexMap::new();
    choice.insert("finish_reason".to_owned(), finish);
    choice.insert("index".to_owned(), index(fields.get("index"), kind)?);
    match kind {
        ChoiceKind::Model => {
            choice.insert("message".to_owned(), message(fields.get("message"))?);
            if let Some(logprobs) = fields
                .get("logprobs")
                .filter(|value| !matches!(value, Json::Null))
            {
                choice.insert("logprobs".to_owned(), logprobs.clone());
            }
            if let Some(provider) = provider_fields(fields, native)? {
                choice.insert("provider_specific_fields".to_owned(), provider);
            }
            copy_extra(fields, &MODEL_FIELDS, &mut choice);
        }
        ChoiceKind::Text => {
            for key in ["text", "logprobs"] {
                choice.insert(
                    key.to_owned(),
                    fields.get(key).cloned().unwrap_or(Json::Null),
                );
            }
            copy_extra(fields, &TEXT_FIELDS, &mut choice);
        }
    }
    Ok(Json::Object(choice))
}

fn finish_reason(
    value: Option<&Json>,
    kind: ChoiceKind,
) -> Result<(Json, Option<String>), PayloadError> {
    let fallback = match kind {
        ChoiceKind::Model => Json::Text("stop".to_owned()),
        ChoiceKind::Text => Json::Null,
    };
    match value {
        None | Some(Json::Null) => Ok((fallback, None)),
        Some(Json::Text(reason)) if reason.is_empty() => Ok((fallback, None)),
        Some(Json::Text(reason)) => {
            let mapped = map_finish_reason(reason);
            let native = (kind == ChoiceKind::Model && mapped != reason).then(|| reason.clone());
            Ok((Json::Text(mapped.to_owned()), native))
        }
        Some(_) => Err(PayloadError::WrongType(
            "choices[].finish_reason",
            "a string or null",
        )),
    }
}

fn index(value: Option<&Json>, kind: ChoiceKind) -> Result<Json, PayloadError> {
    let zero = Json::Number(Number::from(0_u8));
    match value {
        Some(present) if kind == ChoiceKind::Text => Ok(present.clone()),
        Some(Json::Number(number)) if number.is_i64() || number.is_u64() => {
            Ok(Json::Number(number.clone()))
        }
        None | Some(Json::Null) => Ok(zero),
        Some(_) => Err(PayloadError::WrongType("choices[].index", "an integer")),
    }
}

fn provider_fields(
    fields: &IndexMap<String, Json>,
    native: Option<String>,
) -> Result<Option<Json>, PayloadError> {
    let mut provider = match fields.get("provider_specific_fields") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(provider)) => provider.clone(),
        Some(_) => {
            return Err(PayloadError::WrongType(
                "choices[].provider_specific_fields",
                "a JSON object or null",
            ));
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

fn message(value: Option<&Json>) -> Result<Json, PayloadError> {
    let empty: IndexMap<String, Json> = IndexMap::new();
    let fields = match value {
        None | Some(Json::Null) => &empty,
        Some(Json::Object(fields)) => fields,
        Some(_) => {
            return Err(PayloadError::WrongType(
                "choices[].message",
                "a JSON object",
            ));
        }
    };
    let mut message = IndexMap::from([
        ("content".to_owned(), Json::Null),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("tool_calls".to_owned(), Json::Null),
        ("function_call".to_owned(), Json::Null),
        ("provider_specific_fields".to_owned(), Json::Null),
    ]);
    for (key, item) in fields {
        let dropped = matches!(item, Json::Null)
            && (key == "role" || DROPPED_WHEN_NULL.contains(&key.as_str()));
        if !dropped {
            message.insert(key.clone(), item.clone());
        }
    }
    normalize_message(&mut message)?;
    Ok(Json::Object(message))
}

fn normalize_message(message: &mut IndexMap<String, Json>) -> Result<(), PayloadError> {
    if let Some(role) = message.get("role")
        && !matches!(role, Json::Text(_))
    {
        return Err(PayloadError::WrongType(
            "choices[].message.role",
            "a string or null",
        ));
    }
    let calls = match message.get("tool_calls") {
        Some(Json::List(calls)) if calls.is_empty() => Json::Null,
        Some(calls @ Json::List(_)) => calls.clone(),
        None | Some(Json::Null) => Json::Null,
        Some(_) => {
            return Err(PayloadError::WrongType(
                "choices[].message.tool_calls",
                "a JSON array or null",
            ));
        }
    };
    message.insert("tool_calls".to_owned(), calls);
    for (key, field) in [
        ("function_call", "choices[].message.function_call"),
        (
            "provider_specific_fields",
            "choices[].message.provider_specific_fields",
        ),
    ] {
        let value = match message.get(key) {
            Some(object @ Json::Object(_)) => object.clone(),
            None | Some(Json::Null) => Json::Null,
            Some(_) => {
                return Err(PayloadError::WrongType(field, "a JSON object or null"));
            }
        };
        message.insert(key.to_owned(), value);
    }
    Ok(())
}

fn copy_extra(
    fields: &IndexMap<String, Json>,
    declared: &[&str],
    out: &mut IndexMap<String, Json>,
) {
    for (key, value) in fields {
        if !declared.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
}
