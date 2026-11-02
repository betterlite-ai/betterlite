use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

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

// A present-but-empty finish_reason (0, "", false, []) collapses to "stop" too: TextChoices only keeps it when truthy.
fn is_blank_finish_reason(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Number(number) => number.as_f64() == Some(0.0_f64),
        Value::String(text) => text.is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::Bool(true) => false,
    }
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
            self.finish_reason
                .filter(|value| !is_blank_finish_reason(value))
                .unwrap_or_else(|| Value::String("stop".to_owned())),
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
) -> Result<Map<String, Value>, ReplyError> {
    let (Some(response), Some(mut target)) = (response_object, model_response_object) else {
        return Err(ReplyError::MissingResponseFormat);
    };
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
    let mut hidden_params = match target.remove("_hidden_params") {
        Some(Value::Object(existing)) => existing,
        _ => Map::new(),
    };
    hidden_params.insert("original_response".to_owned(), response);
    target.insert("_hidden_params".to_owned(), Value::Object(hidden_params));
    Ok(target)
}
