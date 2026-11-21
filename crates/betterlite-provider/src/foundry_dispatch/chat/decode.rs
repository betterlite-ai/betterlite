use std::io::Error as IoError;

use betterlite_foundry::chat::response::{ResponseError, transform_response};
use betterlite_prompts::arguments::json::Json;

use crate::operation::{ChatCompletion, OperationError, OperationResult, success};
use crate::prepared::{PreparedKind, PreparedRequest, Upstream};

pub fn decode(
    prepared: &PreparedRequest,
    upstream: Upstream,
) -> Result<OperationResult, OperationError> {
    match (prepared.kind(), upstream) {
        (PreparedKind::Chat(chat), Upstream::Buffered(response)) => {
            let checked = success(response)?;
            let body: Json =
                serde_json::from_slice(&checked.body).map_err(|source| OperationError::Decode {
                    operation: "chat",
                    source,
                })?;
            let model = chat.client_model.as_deref().unwrap_or_default();
            let completion = transform_response(model, body).map_err(decode_error)?;
            Ok(OperationResult::Chat(ChatCompletion::OpenAi(Box::new(
                completion,
            ))))
        }
        (PreparedKind::Stream(_), Upstream::Streaming(response)) => {
            Ok(OperationResult::Stream(Box::new(response)))
        }
        (PreparedKind::Stream(_), Upstream::Buffered(response)) => {
            success(response)?;
            Err(unsupported())
        }
        (PreparedKind::Chat(_) | PreparedKind::Stream(_), Upstream::Anthropic(_))
        | (PreparedKind::Chat(_), Upstream::Streaming(_))
        | (
            PreparedKind::Embedding(_) | PreparedKind::Other,
            Upstream::Buffered(_) | Upstream::Streaming(_) | Upstream::Anthropic(_),
        ) => Err(unsupported()),
    }
}

#[must_use]
pub fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("azure_ai"),
        operation: String::from("provider endpoint"),
    }
}

fn decode_error(error: ResponseError) -> OperationError {
    match error {
        ResponseError::Json(source) => OperationError::Decode {
            operation: "chat",
            source,
        },
        other @ (ResponseError::NotAnObject | ResponseError::ModelNotString) => {
            OperationError::Decode {
                operation: "chat",
                source: serde_json::Error::io(IoError::other(other.to_string())),
            }
        }
    }
}
