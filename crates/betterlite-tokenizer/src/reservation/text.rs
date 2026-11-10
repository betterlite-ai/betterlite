use betterlite_models::decisions::Json;

use crate::count::Counter;
use crate::count::request::{TextInput, TokenRequest};

use super::ReservationError;
use super::python_text::{Style, render, str_of};

// LiteLLM's _count_text_tokens: a stack walk, so untrusted nesting cannot recurse; each item counts on its own
pub(crate) async fn count_text_tokens(
    counter: &Counter,
    model: &str,
    value: &Json,
) -> Result<usize, ReservationError> {
    let mut total = 0_usize;
    let mut stack = vec![value];
    while let Some(item) = stack.pop() {
        let text = match item {
            Json::Null => continue,
            Json::List(items) => {
                stack.extend(items);
                continue;
            }
            Json::Object(_) => render(item, Style::Json),
            other @ (Json::Bool(_) | Json::Number(_) | Json::Text(_)) => str_of(other),
        };
        let tokens = tokenize(counter, model, &text).await?;
        total = total
            .checked_add(tokens)
            .ok_or(ReservationError::Overflow)?;
    }
    Ok(total)
}

async fn tokenize(counter: &Counter, model: &str, text: &str) -> Result<usize, ReservationError> {
    let input = TextInput::One(text.to_owned());
    let request = TokenRequest {
        model,
        custom_tokenizer: None,
        text: Some(&input),
        messages: None,
        count_response_tokens: None,
        tools: None,
        tool_choice: None,
    };
    counter
        .count(&request)
        .await
        .map_err(ReservationError::from)
}
