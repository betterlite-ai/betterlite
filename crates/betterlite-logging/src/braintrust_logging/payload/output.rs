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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChoiceKind {
    Model,
    Text,
}

#[derive(Clone, Copy)]
enum Expected {
    Object,
    Array,
    Text,
    Integer,
}

impl Expected {
    fn matches(self, value: &Json) -> bool {
        match (self, value) {
            (Self::Object, Json::Object(_))
            | (Self::Array, Json::List(_))
            | (Self::Text, Json::Text(_)) => true,
            (Self::Integer, Json::Number(number)) => number.is_i64() || number.is_u64(),
            (Self::Object | Self::Array | Self::Text | Self::Integer, _) => false,
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::Object => "a JSON object",
            Self::Array => "a JSON array",
            Self::Text => "a string",
            Self::Integer => "an integer",
        }
    }
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
        ChoiceKind::Model => model_fields(fields, native, &mut choice)?,
        ChoiceKind::Text => {
            choice.insert(
                "text".to_owned(),
                fields.get("text").cloned().unwrap_or(Json::Null),
            );
            choice.insert(
                "logprobs".to_owned(),
                fields.get("logprobs").cloned().unwrap_or(Json::Null),
            );
            extras(fields, &TEXT_CHOICE_FIELDS, &mut choice);
        }
    }
    Ok(Json::Object(choice))
}

fn model_fields(
    fields: &IndexMap<String, Json>,
    native: Option<String>,
    choice: &mut IndexMap<String, Json>,
) -> Result<(), PayloadError> {
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
    extras(fields, &MODEL_CHOICE_FIELDS, choice);
    Ok(())
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
        Some(_) => Err(PayloadError::Field {
            field: "choices[].finish_reason",
            expected: "a string or null",
        }),
    }
}

fn index(value: Option<&Json>, kind: ChoiceKind) -> Result<Json, PayloadError> {
    let zero = Json::Number(Number::from(0_u8));
    match kind {
        ChoiceKind::Model => checked(value, "choices[].index", Expected::Integer, zero),
        ChoiceKind::Text => Ok(value.cloned().unwrap_or(zero)),
    }
}

fn checked(
    value: Option<&Json>,
    field: &'static str,
    expected: Expected,
    fallback: Json,
) -> Result<Json, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(fallback),
        Some(present) if expected.matches(present) => Ok(present.clone()),
        Some(_) => Err(PayloadError::Field {
            field,
            expected: expected.describe(),
        }),
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
    message_body(fields)
}

fn message_body(fields: &IndexMap<String, Json>) -> Result<Json, PayloadError> {
    let mut message = IndexMap::new();
    message.insert(
        "content".to_owned(),
        fields.get("content").cloned().unwrap_or(Json::Null),
    );
    message_defaults(fields, &mut message)?;
    extras(fields, &MESSAGE_FIELDS, &mut message);
    message.retain(|key, item| {
        !(matches!(item, Json::Null) && DROPPED_WHEN_NULL.contains(&key.as_str()))
    });
    Ok(Json::Object(message))
}

fn message_defaults(
    fields: &IndexMap<String, Json>,
    message: &mut IndexMap<String, Json>,
) -> Result<(), PayloadError> {
    message.insert(
        "role".to_owned(),
        checked(
            fields.get("role"),
            "choices[].message.role",
            Expected::Text,
            Json::Text("assistant".to_owned()),
        )?,
    );
    let calls = match fields.get("tool_calls") {
        Some(Json::List(calls)) if calls.is_empty() => None,
        other => other,
    };
    message.insert(
        "tool_calls".to_owned(),
        checked(
            calls,
            "choices[].message.tool_calls",
            Expected::Array,
            Json::Null,
        )?,
    );
    for (key, name) in [
        ("function_call", "choices[].message.function_call"),
        (
            "provider_specific_fields",
            "choices[].message.provider_specific_fields",
        ),
    ] {
        message.insert(
            key.to_owned(),
            checked(fields.get(key), name, Expected::Object, Json::Null)?,
        );
    }
    Ok(())
}

fn extras(fields: &IndexMap<String, Json>, declared: &[&str], out: &mut IndexMap<String, Json>) {
    for (key, value) in fields {
        if !declared.contains(&key.as_str()) {
            out.insert(key.clone(), value.clone());
        }
    }
}
