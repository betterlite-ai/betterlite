use betterlite_vertex::gemini_embedding_call::plan::flatten_and_detect_file_refs;
use betterlite_vertex::gemini_embedding_records::input::{EmbeddingInput, InputEntry};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    inputs: Vec<Value>,
}

// betterlite's input holds text only, so the helper drops the token ids LiteLLM's flatten ignores
fn declared(value: &Value) -> EmbeddingInput {
    match value {
        Value::String(text) => EmbeddingInput::Text(text.clone()),
        Value::Array(items) => EmbeddingInput::Entries(
            items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(InputEntry::Text(text.clone())),
                    Value::Array(group) => Some(InputEntry::Group(
                        group
                            .iter()
                            .filter_map(|element| element.as_str().map(str::to_owned))
                            .collect(),
                    )),
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
                })
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            EmbeddingInput::Entries(Vec::new())
        }
    }
}

pub(super) fn run(input: &Input) -> Value {
    let mut calls = Vec::with_capacity(input.inputs.len());
    for value in &input.inputs {
        let parsed = declared(value);
        let (elements, has_file_refs) = flatten_and_detect_file_refs(&parsed);
        calls.push(json!({"elements": elements, "has_file_refs": has_file_refs}));
    }
    json!({"calls": calls})
}
