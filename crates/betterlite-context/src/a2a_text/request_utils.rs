use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{TextInput, TokenRequest};

use super::{A2aResponse, A2aTextError, Field, ListValue, RecordOr, RecordValue, Shape};

mod tests;

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
        Some(RecordOr::Record(record)) => text_parts(&record.parts),
    }
}

pub fn extract_text_from_response(response: &A2aResponse) -> Result<String, A2aTextError> {
    let Some(RecordOr::Record(result)) = &response.result else {
        return Ok(String::new());
    };
    if matches!(&result.kind, Field::Present(Shape::Text(kind)) if kind == "message") {
        return text_parts(&result.parts);
    }
    result.message.as_ref().map_or_else(
        || Ok(String::new()),
        |message| extract_text_from_message(Some(message)),
    )
}

pub async fn usage_from_request_response(
    counter: &Counter,
    message: Option<&RecordValue>,
    response: &A2aResponse,
) -> Result<(usize, usize, usize), A2aTextError> {
    let prompt_text = extract_text_from_message(message)?;
    let completion_text = extract_text_from_response(response)?;
    let prompt_tokens = count_tokens(counter, &prompt_text).await;
    let completion_tokens = count_tokens(counter, &completion_text).await;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(A2aTextError::UsageOverflow)?;
    Ok((prompt_tokens, completion_tokens, total_tokens))
}

fn text_parts(parts: &Field<ListValue>) -> Result<String, A2aTextError> {
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
        if !matches!(&part.kind, Field::Present(Shape::Text(kind)) if kind == "text") {
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
