use betterlite_prompts::arguments::json::Json;

use super::error::BatchEmbeddingError;

// Titan v2 takes exactly one string per InvokeModel call, so a fan-out is refused instead of guessed
pub fn coerce_input_to_string(
    raw_input: Option<&Json>,
    model: &str,
) -> Result<String, BatchEmbeddingError> {
    let Some(raw) = raw_input.filter(|value| !matches!(value, Json::Null)) else {
        return Err(BatchEmbeddingError::MissingInput {
            model: model.to_owned(),
        });
    };
    let candidate = match raw {
        Json::List(items) => {
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
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) if !number.is_i64() && !number.is_u64() => {
            Err(BatchEmbeddingError::InputNotString {
                found: "float",
                model: model.to_owned(),
            })
        }
        Json::Null => Err(BatchEmbeddingError::InputNotString {
            found: "NoneType",
            model: model.to_owned(),
        }),
        Json::Object(_) => Err(BatchEmbeddingError::InputNotString {
            found: "dict",
            model: model.to_owned(),
        }),
        Json::Bool(_) | Json::List(_) | Json::Number(_) => {
            Err(BatchEmbeddingError::TokenizedInput {
                model: model.to_owned(),
            })
        }
    }
}
