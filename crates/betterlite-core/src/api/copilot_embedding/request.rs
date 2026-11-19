use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Object;
use serde_json::Value;

pub const MODEL_PREFIX: &str = "github_copilot/";

// LiteLLM advertises exactly these four embedding parameters and drops every other request key
pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["timeout", "dimensions", "encoding_format", "user"];

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

#[must_use]
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Object {
    let mut body = Object::from_iter([
        (
            String::from("model"),
            Value::String(model.strip_prefix(MODEL_PREFIX).unwrap_or(model).to_owned()),
        ),
        (String::from("input"), input_value(input)),
    ]);
    // LiteLLM spreads the optional parameters last, so they may replace model and input
    body.extend(optional_params.clone());
    body
}

fn input_value(input: &EmbeddingInput) -> Value {
    match input {
        EmbeddingInput::Text(text) => Value::Array(vec![Value::String(text.clone())]),
        EmbeddingInput::Texts(texts) => {
            Value::Array(texts.iter().cloned().map(Value::String).collect())
        }
        EmbeddingInput::Tokens(tokens) => {
            Value::Array(tokens.iter().map(|token| Value::from(*token)).collect())
        }
        EmbeddingInput::TokenLists(lists) => Value::Array(
            lists
                .iter()
                .map(|list| Value::Array(list.iter().map(|token| Value::from(*token)).collect()))
                .collect(),
        ),
    }
}
