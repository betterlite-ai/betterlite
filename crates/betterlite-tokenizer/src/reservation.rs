use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::count::Counter;

pub mod error;
mod messages;
mod python_text;
pub mod schedule;
mod tests;
mod text;

use error::ReservationError;
use schedule::{AccountPool, OFFLOAD_MIN_CHARS};

pub type RequestBody = IndexMap<String, Json>;

pub struct ParsedRequest<'request> {
    pub body: &'request RequestBody,
    // the pinned TOKEN_COUNTER route rollout is PYTHON_ONLY, so raw bytes never change a count
    pub raw_body: Option<&'request [u8]>,
}

// LiteLLM's _INPUT_SIZE_FIELDS, the fields count_input_tokens_for_model hands the tokenizer
const INPUT_SIZE_FIELDS: [&str; 7] = [
    "messages",
    "prompt",
    "input",
    "query",
    "documents",
    "tools",
    "tool_choice",
];

// Length of the request's input text in Unicode code points, a cheap stand-in for tokenizing cost
#[must_use]
pub fn approximate_input_size(body: &RequestBody) -> usize {
    INPUT_SIZE_FIELDS.iter().fold(0_usize, |total, field| {
        let length = body
            .get(*field)
            .map_or(0, |value| python_text::str_of(value).chars().count());
        total.saturating_add(length)
    })
}

// LiteLLM's count_input_tokens: each model counts on its own, a failed count is left out and a repeat keeps its first place
pub async fn count_input_tokens(
    counter: &Arc<Counter>,
    request: ParsedRequest<'_>,
    models: &[String],
    pool: &dyn AccountPool,
) -> IndexMap<String, usize> {
    if models.is_empty() {
        return IndexMap::new();
    }
    if approximate_input_size(request.body) < OFFLOAD_MIN_CHARS {
        return count_models(counter.as_ref(), request.body, models).await;
    }
    let owned_body = request.body.clone();
    let owned_models = models.to_vec();
    let owned_counter = Arc::clone(counter);
    let job =
        Box::pin(
            async move { count_models(owned_counter.as_ref(), &owned_body, &owned_models).await },
        );
    match pool.run(job).await {
        Ok(counts) => counts,
        Err(error) => {
            tracing::debug!(error = %error, "the reservation input count was not offloaded");
            count_models(counter.as_ref(), request.body, models).await
        }
    }
}

// LiteLLM's count_input_tokens_for_model, None where it would have swallowed an exception
pub async fn count_input_tokens_for_model(
    counter: &Counter,
    body: &RequestBody,
    model: &str,
) -> Option<usize> {
    match count_model(counter, body, model).await {
        Ok(count) => count,
        Err(error) => {
            tracing::debug!(error = %error, "unable to count input tokens for budget reservation");
            None
        }
    }
}

async fn count_models(
    counter: &Counter,
    body: &RequestBody,
    models: &[String],
) -> IndexMap<String, usize> {
    let mut counts = IndexMap::new();
    for model in models {
        if let Some(tokens) = count_input_tokens_for_model(counter, body, model).await {
            counts.insert(model.clone(), tokens);
        }
    }
    counts
}

async fn count_model(
    counter: &Counter,
    body: &RequestBody,
    model: &str,
) -> Result<Option<usize>, ReservationError> {
    if let Some(messages) = body.get("messages") {
        return messages::count_messages_field(counter, body, model, messages).await;
    }
    if let Some(prompt) = body.get("prompt") {
        return Ok(Some(text::count_text_tokens(counter, model, prompt).await?));
    }
    if let Some(input) = body.get("input") {
        return Ok(Some(text::count_text_tokens(counter, model, input).await?));
    }
    if body.contains_key("query") || body.contains_key("documents") {
        return count_query_and_documents(counter, body, model).await;
    }
    Ok(None)
}

async fn count_query_and_documents(
    counter: &Counter,
    body: &RequestBody,
    model: &str,
) -> Result<Option<usize>, ReservationError> {
    let query = match body.get("query") {
        Some(value) => text::count_text_tokens(counter, model, value).await?,
        None => 0,
    };
    let documents = match body.get("documents") {
        Some(value) => text::count_text_tokens(counter, model, value).await?,
        None => 0,
    };
    Ok(Some(
        query
            .checked_add(documents)
            .ok_or(ReservationError::Overflow)?,
    ))
}
