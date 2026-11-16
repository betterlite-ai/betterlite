use serde::Deserialize;
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod chat;
mod embedding;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ChatSupported { calls: Vec<chat::SupportedCall> },
    ChatMap { calls: Vec<chat::MapCall> },
    ChatMessages { calls: Vec<chat::MessagesCall> },
    EmbeddingHeaders { calls: Vec<embedding::HeadersCall> },
    EmbeddingUrl { calls: Vec<embedding::UrlCall> },
    EmbeddingRequest { calls: Vec<embedding::RequestCall> },
    EmbeddingResponse { calls: Vec<embedding::ResponseCall> },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ChatSupported { calls } => Ok(Value::Array(
                calls.iter().map(chat::supported_value).collect(),
            )),
            Input::ChatMap { calls } => collect(calls, chat::map_value),
            Input::ChatMessages { calls } => collect(calls, chat::messages_value),
            Input::EmbeddingHeaders { calls } => collect(calls, embedding::headers_value),
            Input::EmbeddingUrl { calls } => collect(calls, embedding::url_value),
            Input::EmbeddingRequest { calls } => Ok(Value::Array(
                calls.iter().map(embedding::request_value).collect(),
            )),
            Input::EmbeddingResponse { calls } => collect(calls, embedding::response_value),
        }
    }
}

fn collect<Call>(
    calls: &[Call],
    make: fn(&Call) -> Result<Value, Box<HelperError>>,
) -> HelperResult {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(make(call)?);
    }
    Ok(Value::Array(values))
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
