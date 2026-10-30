use indexmap::IndexMap;
use serde::Deserialize;

use betterlite_error::error::{ErrorClass, ErrorContext};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
#[serde(untagged)]
pub enum JsonValue {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Self>),
    Object(IndexMap<String, Self>),
}

type JsonMap = IndexMap<String, JsonValue>;

impl JsonValue {
    const fn as_object(&self) -> Option<&JsonMap> {
        match self {
            Self::Object(values) => Some(values),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) | Self::Array(_) => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPolicyInput {
    pub message: String,
    pub model: String,
    pub extra_information: String,
    #[serde(default)]
    pub body: JsonValue,
    #[serde(default)]
    pub response: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ContentPolicyError {
    #[serde(skip)]
    pub classification: ErrorClass,
    pub class: &'static str,
    pub status: u16,
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: JsonValue,
    pub code: JsonValue,
    pub param: JsonValue,
    pub model: String,
    pub provider: &'static str,
    pub litellm_debug_info: String,
    pub response: Option<JsonValue>,
    pub body: JsonMap,
    pub provider_specific_fields: JsonMap,
}

#[must_use]
pub fn content_policy_error(input: &ContentPolicyInput) -> ContentPolicyError {
    let (azure_error, inner_error) = extract_azure_error(&input.body);
    let provider_message = provider_field(
        &azure_error,
        "message",
        JsonValue::String(input.message.clone()),
    );
    let provider_type = provider_field(
        &azure_error,
        "type",
        JsonValue::String("invalid_request_error".to_owned()),
    );
    let provider_code = provider_field(
        &azure_error,
        "code",
        JsonValue::String("content_policy_violation".to_owned()),
    );
    let body = openai_body(&provider_message, &provider_type, &provider_code);
    let provider_specific_fields = azure_extensions(azure_error, inner_error);
    let message = violation_message(&provider_message);
    ContentPolicyError {
        classification: classification(&message, &input.model, &provider_type, &provider_code),
        class: "ContentPolicyViolation",
        status: 400,
        message,
        error_type: provider_type,
        code: provider_code,
        param: JsonValue::Null,
        model: input.model.clone(),
        provider: "azure",
        litellm_debug_info: input.extra_information.clone(),
        response: input.response.clone(),
        body,
        provider_specific_fields,
    }
}

fn provider_field(error: &JsonMap, name: &str, fallback: JsonValue) -> JsonValue {
    let chosen = error.get(name).filter(|value| is_truthy(value)).cloned();
    chosen.unwrap_or(fallback)
}

fn openai_body(message: &JsonValue, error_type: &JsonValue, code: &JsonValue) -> JsonMap {
    let mut body = JsonMap::new();
    body.extend([
        ("message".to_owned(), message.clone()),
        ("type".to_owned(), error_type.clone()),
        ("code".to_owned(), code.clone()),
        ("param".to_owned(), JsonValue::Null),
    ]);
    body
}

fn azure_extensions(error: JsonMap, inner_error: Option<JsonValue>) -> JsonMap {
    let azure_error = if error.is_empty() {
        JsonValue::Null
    } else {
        JsonValue::Object(error)
    };
    let inner_value = inner_error.unwrap_or(JsonValue::Null);
    JsonMap::from_iter([
        ("innererror".to_owned(), inner_value.clone()),
        ("inner_error".to_owned(), inner_value),
        ("azure_error".to_owned(), azure_error),
    ])
}

fn violation_message(message: &JsonValue) -> String {
    let rendered = display_message(message);
    format!("litellm.BadRequestError: litellm.ContentPolicyViolationError: {rendered}")
}

fn classification(
    message: &str,
    model: &str,
    error_type: &JsonValue,
    code: &JsonValue,
) -> ErrorClass {
    let context = ErrorContext {
        message: message.to_owned(),
        model: Some(model.to_owned()),
        provider: Some("azure".to_owned()),
        max_retries: None,
        num_retries: None,
        body_type: error_type.as_str().map(str::to_owned),
        body_param: None,
        body_code: code.as_str().map(str::to_owned),
    };
    ErrorClass::ContentPolicyViolation(Box::new(context))
}

#[must_use]
pub fn extract_azure_error(body: &JsonValue) -> (JsonMap, Option<JsonValue>) {
    let Some(body_object) = body.as_object() else {
        return (JsonMap::new(), None);
    };
    let azure_error = match body_object.get("error") {
        Some(JsonValue::Object(error)) => error.clone(),
        _ => body_object.clone(),
    };
    let inner_error = [
        azure_error.get("inner_error"),
        azure_error.get("innererror"),
        body_object.get("innererror"),
        body_object.get("inner_error"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_some_and(is_truthy))
    .flatten()
    .cloned()
    .or_else(|| body_object.get("inner_error").cloned());
    (azure_error, inner_error)
}

fn is_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null | JsonValue::Bool(false) => false,
        JsonValue::Number(number) => *number != 0.0_f64,
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
        JsonValue::Bool(true) => true,
    }
}

fn display_message(value: &JsonValue) -> String {
    match value {
        JsonValue::String(message) => message.clone(),
        JsonValue::Array(_)
        | JsonValue::Object(_)
        | JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_) => python_repr(value),
    }
}

fn python_repr(value: &JsonValue) -> String {
    if let JsonValue::String(text) = value {
        return python_string_repr(text);
    }
    if let JsonValue::Array(values) = value {
        let items = values
            .iter()
            .map(python_repr)
            .collect::<Vec<_>>()
            .join(", ");
        return format!("[{items}]");
    }
    if let JsonValue::Object(values) = value {
        let fields = values
            .iter()
            .map(|(key, item)| format!("{}: {}", python_string_repr(key), python_repr(item)))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{{{fields}}}");
    }
    if matches!(value, JsonValue::Null) {
        return "None".to_owned();
    }
    if let JsonValue::Bool(boolean) = value {
        return (if *boolean { "True" } else { "False" }).to_owned();
    }
    if let JsonValue::Number(number) = value {
        return number.to_string();
    }
    String::new()
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(value.len().saturating_add(2));
    rendered.push(quote);
    for character in value.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            quote_character if quote_character == quote => {
                rendered.push('\\');
                rendered.push(quote_character);
            }
            other => rendered.push(other),
        }
    }
    rendered.push(quote);
    rendered
}

mod tests;
