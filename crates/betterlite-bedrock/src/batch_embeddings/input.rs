use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use super::error::BatchEmbeddingError;

// One OpenAI /v1/embeddings `input` value, kept apart so each refusal can name the type it read
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Text(String),
    Number(Number),
    Bool(bool),
    List(Vec<Self>),
    Null,
    Object(IgnoredAny),
}

// Titan v2 takes exactly one string per InvokeModel call, so a fan-out is refused instead of guessed
pub fn coerce_input_to_string(
    raw_input: Option<&EmbeddingInput>,
    model: &str,
) -> Result<String, BatchEmbeddingError> {
    let Some(raw) = raw_input else {
        return Err(BatchEmbeddingError::MissingInput {
            model: model.to_owned(),
        });
    };
    let candidate = match raw {
        EmbeddingInput::List(items) => {
            let [single] = items.as_slice() else {
                return Err(BatchEmbeddingError::MultipleInputs {
                    items: items.len(),
                    model: model.to_owned(),
                });
            };
            single
        }
        other => other,
    };
    // each refusal names the Python type it read, and an int is a token list, True included
    match candidate {
        EmbeddingInput::Text(text) => Ok(text.clone()),
        EmbeddingInput::Number(number) if number.as_str().contains(['.', 'e', 'E']) => {
            Err(BatchEmbeddingError::InputNotString {
                found: "float",
                model: model.to_owned(),
            })
        }
        EmbeddingInput::Null => Err(BatchEmbeddingError::InputNotString {
            found: "NoneType",
            model: model.to_owned(),
        }),
        EmbeddingInput::Object(_) => Err(BatchEmbeddingError::InputNotString {
            found: "dict",
            model: model.to_owned(),
        }),
        EmbeddingInput::Bool(_) | EmbeddingInput::List(_) | EmbeddingInput::Number(_) => {
            Err(BatchEmbeddingError::TokenizedInput {
                model: model.to_owned(),
            })
        }
    }
}
