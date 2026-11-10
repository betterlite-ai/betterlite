use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::text_completion_policy::reply::mapped_finish_reason;

// The OpenAI SDK's Completion model after `.parse()`, before TextCompletionResponse wraps it
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletionReply {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub choices: Vec<CompletionChoice>,
    #[serde(default)]
    pub usage: Option<CompletionUsage>,
    #[serde(default)]
    pub system_fingerprint: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletionChoice {
    pub text: String,
    pub index: i64,
    #[serde(default)]
    pub logprobs: Option<Value>,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletionUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    #[serde(default)]
    pub completion_tokens_details: Option<Value>,
    #[serde(default)]
    pub prompt_tokens_details: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// Python truthiness over the values a response can carry: Usage and TextChoices default blank values.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
        Value::Bool(true) => false,
    }
}

fn normalized_usage(raw: Option<Value>) -> Value {
    let fields = match raw.filter(|value| !is_blank(value)) {
        Some(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let mut usage = fields.clone();
    for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        let count = fields
            .get(key)
            .filter(|value| !is_blank(value))
            .cloned()
            .unwrap_or_else(|| Value::from(0_i64));
        usage.insert(key.to_owned(), count);
    }
    for key in ["completion_tokens_details", "prompt_tokens_details"] {
        let details = fields
            .get(key)
            .filter(|value| !is_blank(value))
            .cloned()
            .unwrap_or(Value::Null);
        usage.insert(key.to_owned(), details);
    }
    Value::Object(usage)
}

// TextChoices.__init__ maps a truthy finish_reason and keeps a blank or missing one as None.
fn normalized_choice(mut fields: Map<String, Value>) -> Value {
    let finish_reason = fields
        .remove("finish_reason")
        .filter(|value| !is_blank(value))
        .map_or(Value::Null, |value| match value {
            Value::String(text) => Value::String(mapped_finish_reason(&text).to_owned()),
            other @ (Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_)) => other,
        });
    fields.insert("finish_reason".to_owned(), finish_reason);
    fields.entry("text".to_owned()).or_insert(Value::Null);
    fields
        .entry("index".to_owned())
        .or_insert_with(|| Value::from(0_i64));
    fields.entry("logprobs".to_owned()).or_insert(Value::Null);
    Value::Object(fields)
}

// TextCompletionResponse.__init__ replaces any choices value that is not a list with one default choice.
fn normalized_choices(raw: Option<Value>) -> Value {
    match raw {
        Some(Value::Array(choices)) => Value::Array(
            choices
                .into_iter()
                .map(|choice| match choice {
                    Value::Object(fields) => normalized_choice(fields),
                    other @ (Value::Null
                    | Value::Bool(_)
                    | Value::Number(_)
                    | Value::String(_)
                    | Value::Array(_)) => other,
                })
                .collect(),
        ),
        _ => Value::Array(vec![Value::Object(Map::from_iter([
            ("text".to_owned(), Value::Null),
            ("finish_reason".to_owned(), Value::Null),
            ("logprobs".to_owned(), Value::Null),
            ("index".to_owned(), Value::from(0_i64)),
        ]))]),
    }
}

// TextCompletionResponse(**response_json).model_dump(): object is forced and Usage/TextChoices default their fields.
#[must_use]
pub fn text_response(mut sdk_dump: Value) -> Value {
    let Value::Object(fields) = &mut sdk_dump else {
        return sdk_dump;
    };
    fields.insert(
        "object".to_owned(),
        Value::String("text_completion".to_owned()),
    );
    let usage = normalized_usage(fields.remove("usage"));
    fields.insert("usage".to_owned(), usage);
    let choices = normalized_choices(fields.remove("choices"));
    fields.insert("choices".to_owned(), choices);
    sdk_dump
}
