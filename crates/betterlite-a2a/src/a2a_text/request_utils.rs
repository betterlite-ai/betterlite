use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};
use serde::Deserialize;
use thiserror::Error;

use super::{A2aResponse, A2aTextError, Field, ListValue, RecordOr, RecordValue, Shape};

mod tests;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RequestUtilsError {
    #[error(transparent)]
    Text(#[from] A2aTextError),
    #[error("A2A usage token total overflows")]
    UsageOverflow,
}

#[derive(Debug, Deserialize)]
pub struct A2aRequest {
    pub params: Option<RecordOr<Box<A2aParams>>>,
}

#[derive(Debug, Deserialize)]
pub struct A2aParams {
    pub message: Option<RecordValue>,
}

#[derive(Debug, Clone, Copy)]
enum PartSource {
    Dict,
    // the a2a-sdk parses request parts into TextPart, whose kind defaults to "text", before model_dump writes the JSON
    SdkDump,
}

#[must_use]
pub async fn count_tokens(counter: &Counter, text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let input = TextInput::One(text.to_owned());
    let request = TokenRequest {
        model: "",
        custom_tokenizer: None,
        text: Some(&input),
        messages: None,
        count_response_tokens: None,
        tools: None,
        tool_choice: None,
    };
    match counter.count(&request).await {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::debug!("Failed to count tokens: {error}");
            0
        }
    }
}

pub fn extract_text_from_message(message: Option<&RecordValue>) -> Result<String, A2aTextError> {
    match message {
        None | Some(RecordOr::Other(_)) => Ok(String::new()),
        Some(RecordOr::Record(record)) => text_parts(&record.parts, PartSource::Dict),
    }
}

#[must_use = "the caller needs the request's input message or the fact that it has none"]
pub fn get_input_message_from_request(request: &A2aRequest) -> Option<&RecordValue> {
    let params = request.params.as_ref()?;
    match params {
        RecordOr::Record(found) => found.message.as_ref(),
        RecordOr::Other(_) => None,
    }
}

pub fn extract_text_from_response(response: &A2aResponse) -> Result<String, A2aTextError> {
    let Some(RecordOr::Record(result)) = &response.result else {
        return Ok(String::new());
    };
    if matches!(&result.kind, Field::Present(Shape::Text(kind)) if kind == "message") {
        return text_parts(&result.parts, PartSource::Dict);
    }
    result.message.as_ref().map_or_else(
        || Ok(String::new()),
        |message| extract_text_from_message(Some(message)),
    )
}

pub async fn usage_from_request_response(
    counter: &Counter,
    request: &A2aRequest,
    response: &A2aResponse,
) -> Result<(usize, usize, usize), RequestUtilsError> {
    let prompt_text = match get_input_message_from_request(request) {
        Some(RecordOr::Record(message)) => text_parts(&message.parts, PartSource::SdkDump)?,
        None | Some(RecordOr::Other(_)) => String::new(),
    };
    let completion_text = extract_text_from_response(response)?;
    let prompt_tokens = count_tokens(counter, &prompt_text).await;
    let completion_tokens = count_tokens(counter, &completion_text).await;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(RequestUtilsError::UsageOverflow)?;
    Ok((prompt_tokens, completion_tokens, total_tokens))
}

fn text_parts(parts: &Field<ListValue>, source: PartSource) -> Result<String, A2aTextError> {
    let items = match parts {
        Field::Present(ListValue::List(items)) => items,
        Field::Present(ListValue::Other(Shape::Null | Shape::Flag(_) | Shape::Number(_))) => {
            return Err(A2aTextError::PartsNotIterable);
        }
        // LiteLLM iterates a string or object here and skips every element that is not a dict
        Field::Absent | Field::Present(ListValue::Other(_)) => return Ok(String::new()),
    };
    let mut texts = Vec::new();
    for item in items {
        let RecordOr::Record(part) = item else {
            continue;
        };
        let is_text = match (&part.kind, source) {
            (Field::Present(Shape::Text(kind)), _) => kind == "text",
            (Field::Absent, PartSource::SdkDump) => {
                matches!(&part.text, Field::Present(Shape::Text(_)))
            }
            (Field::Absent | Field::Present(_), _) => false,
        };
        if !is_text {
            continue;
        }
        texts.push(match &part.text {
            Field::Absent => String::new(),
            Field::Present(Shape::Text(text)) => text.clone(),
            Field::Present(_) => return Err(A2aTextError::TextNotString),
        });
    }
    Ok(texts.join(" "))
}
