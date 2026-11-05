use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;
use uuid::Uuid;

mod usage;
use usage::{original_usage_shape, resolve_usage};

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("Error in response object format")]
    MissingResponseFormat,
    #[error("{0}")]
    MalformedChoices(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Deserialize)]
struct TextCompletionChoice {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    finish_reason: Option<Value>,
    #[serde(default)]
    logprobs: Option<Value>,
}

// Python truthiness: 0, "", false, [] and {} are all blank, matching the `if value:` checks in TextChoices and Usage.
fn is_blank(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::String(text) => text.is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::Bool(true) => false,
    }
}

// Ported from _FINISH_REASON_MAP in litellm/litellm_core_utils/core_helpers.py, defaulting to "stop" when the raw value has no entry.
pub(crate) fn mapped_finish_reason(raw: &str) -> &'static str {
    match raw {
        "max_tokens" | "MAX_TOKENS" | "compaction" | "length" => "length",
        "tool_use" | "tool_calls" => "tool_calls",
        "refusal"
        | "ERROR_TOXIC"
        | "SAFETY"
        | "RECITATION"
        | "LANGUAGE"
        | "OTHER"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "NO_IMAGE"
        | "IMAGE_RECITATION"
        | "IMAGE_OTHER"
        | "ESCALATION"
        | "sensitive"
        | "guardrail_intervened"
        | "content_filter"
        | "content_filtered" => "content_filter",
        "function_call" => "function_call",
        _ => "stop",
    }
}

// Mirrors TextChoices.__init__: a blank or missing raw value stays None rather than falling back to "stop".
fn embedded_finish_reason(raw: Option<Value>) -> Value {
    match raw.filter(|value| !is_blank(value)) {
        Some(Value::String(text)) => Value::String(mapped_finish_reason(&text).to_owned()),
        Some(_) => Value::String("stop".to_owned()),
        None => Value::Null,
    }
}

// Choices.__init__ runs the same mapping but defaults a None finish_reason to "stop" instead of keeping it.
fn resolve_finish_reason(raw: Option<Value>) -> Value {
    let mapped = embedded_finish_reason(raw);
    if mapped.is_null() {
        Value::String("stop".to_owned())
    } else {
        mapped
    }
}

// The copy already on target.usage stays trimmed; only the stored original response gets the full wrapper shape.
fn expand_original_usage(response: &mut Value) {
    let Some(fields) = response.as_object_mut() else {
        return;
    };
    let Some(usage) = fields.get("usage").cloned() else {
        return;
    };
    fields.insert("usage".to_owned(), original_usage_shape(&usage));
}

// TextChoices maps finish_reason at parse time, so the stored original response carries the mapped value too.
fn embed_original_choices(response: &mut Value) {
    let Some(Value::Array(choices)) = response.get_mut("choices") else {
        return;
    };
    for choice in choices {
        let Value::Object(fields) = choice else {
            continue;
        };
        let raw = fields.remove("finish_reason");
        fields.insert("finish_reason".to_owned(), embedded_finish_reason(raw));
    }
}

// TextCompletionResponse.__init__: a choices value that isn't a list (missing, null, or any other shape) becomes one default choice.
fn resolve_choices(raw: Option<Value>) -> Value {
    if let Some(Value::Array(choices)) = raw {
        return Value::Array(choices);
    }
    let mut choice = Map::new();
    choice.insert("text".to_owned(), Value::Null);
    choice.insert("finish_reason".to_owned(), Value::Null);
    choice.insert("logprobs".to_owned(), Value::Null);
    choice.insert("index".to_owned(), Value::from(0_i64));
    Value::Array(vec![Value::Object(choice)])
}

// TextCompletionResponse.__init__: id is only generated when missing or null, unlike usage's truthiness check.
fn resolve_id(raw: Option<Value>, identifier: &dyn Fn() -> Uuid) -> Value {
    match raw {
        Some(Value::Null) | None => Value::String(format!("chatcmpl-{}", identifier())),
        Some(value) => value,
    }
}

// TextCompletionResponse.__init__: created is only generated when missing or null, matching resolve_id's strictness.
fn resolve_created(raw: Option<Value>, clock: &dyn Fn() -> i64) -> Value {
    match raw {
        Some(Value::Null) | None => Value::from(clock()),
        Some(value) => value,
    }
}

// Applies TextCompletionResponse's own defaulting before convert_to_chat_model_response_object ever sees the response.
fn normalize_response(
    response: &mut Value,
    identifier: &dyn Fn() -> Uuid,
    clock: &dyn Fn() -> i64,
) {
    let Some(fields) = response.as_object_mut() else {
        return;
    };
    let id = resolve_id(fields.remove("id"), identifier);
    fields.insert("id".to_owned(), id);
    let usage = resolve_usage(fields.remove("usage"));
    fields.insert("usage".to_owned(), usage);
    let choices = resolve_choices(fields.remove("choices"));
    fields.insert("choices".to_owned(), choices);
    // TextCompletionResponse.__init__ always sets object to "text_completion", overwriting whatever the provider sent.
    fields.insert(
        "object".to_owned(),
        Value::String("text_completion".to_owned()),
    );
    let created = resolve_created(fields.remove("created"), clock);
    fields.insert("created".to_owned(), created);
}

impl TextCompletionChoice {
    // Message/Choices always carry tool_calls/function_call/finish_reason (default "stop"), but logprobs only when given.
    fn into_chat_choice(self, index: usize) -> Value {
        let mut message = Map::new();
        message.insert("role".to_owned(), Value::String("assistant".to_owned()));
        message.insert(
            "content".to_owned(),
            self.text.map_or(Value::Null, Value::String),
        );
        message.insert("tool_calls".to_owned(), Value::Null);
        message.insert("function_call".to_owned(), Value::Null);
        let mut choice = Map::new();
        choice.insert(
            "finish_reason".to_owned(),
            resolve_finish_reason(self.finish_reason),
        );
        choice.insert("index".to_owned(), Value::from(index));
        choice.insert("message".to_owned(), Value::Object(message));
        // A null logprobs resolves to None in LiteLLM's Choices, which deletes the field just like an absent key.
        if let Some(logprobs) = self.logprobs.filter(|value| !value.is_null()) {
            choice.insert("logprobs".to_owned(), logprobs);
        }
        Value::Object(choice)
    }
}

// Rebuilds choices with a sequential index, since the text completion response's own index is ignored.
pub fn to_chat_model_response(
    response_object: Option<Value>,
    model_response_object: Option<Map<String, Value>>,
    identifier: &dyn Fn() -> Uuid,
    clock: &dyn Fn() -> i64,
) -> Result<Map<String, Value>, ReplyError> {
    let (Some(mut response), Some(mut target)) = (response_object, model_response_object) else {
        return Err(ReplyError::MissingResponseFormat);
    };
    normalize_response(&mut response, identifier, clock);
    let choices_value = response.get("choices").cloned().unwrap_or(Value::Null);
    let choices: Vec<TextCompletionChoice> = serde_json::from_value(choices_value)?;
    let chat_choices = choices
        .into_iter()
        .enumerate()
        .map(|(index, choice)| choice.into_chat_choice(index))
        .collect();
    target.insert("choices".to_owned(), Value::Array(chat_choices));
    // A real TextCompletionResponse always carries id/model/usage (auto-generated id, nullable model, zero-valued usage), so these always replace the fresh target's own values.
    for key in ["usage", "id", "model"] {
        target.insert(
            key.to_owned(),
            response.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    expand_original_usage(&mut response);
    embed_original_choices(&mut response);
    let mut hidden_params = match target.remove("_hidden_params") {
        Some(Value::Object(existing)) => existing,
        _ => Map::new(),
    };
    hidden_params.insert("original_response".to_owned(), response);
    target.insert("_hidden_params".to_owned(), Value::Object(hidden_params));
    Ok(target)
}
