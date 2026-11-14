use crate::message_normalization::content_to_text;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::error::Error;

const MAX_EMBEDDING_CHARS: usize = 30_000;
const EMPTY_TEXT_PLACEHOLDER: &str = "empty";

#[derive(Clone, Copy)]
pub struct EmbeddingCall<'call> {
    pub parameters: &'call Map<String, Value>,
}

#[async_trait]
pub trait EmbeddingExecutor: Send + Sync {
    type Failure: Error + Send + Sync + 'static;

    async fn embed(&self, call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure>;
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingScoringError<ExecutorFailure>
where
    ExecutorFailure: Error + 'static,
{
    #[error("the embedding call failed")]
    Executor(#[source] ExecutorFailure),
    #[error("the embedding response carried no vectors")]
    EmptyResponse,
}

#[derive(Clone, Copy)]
pub struct EmbeddingScoringRequest<'request> {
    pub query: &'request str,
    pub messages: &'request [Value],
    pub model: &'request str,
    pub embedding_model_params: Option<&'request Map<String, Value>>,
}

pub async fn embedding_score_messages<Cache, Executor>(
    request: EmbeddingScoringRequest<'_>,
    cache: Option<&Cache>,
    executor: &Executor,
) -> Result<Vec<f64>, EmbeddingScoringError<Executor::Failure>>
where
    Cache: Sync + ?Sized,
    Executor: EmbeddingExecutor,
{
    let parameters = embedding_parameters(&request, cache.is_some());
    let embeddings = executor
        .embed(EmbeddingCall {
            parameters: &parameters,
        })
        .await
        .map_err(EmbeddingScoringError::Executor)?;
    let query_embedding = embeddings
        .first()
        .ok_or(EmbeddingScoringError::EmptyResponse)?;
    Ok(embeddings
        .iter()
        .skip(1)
        .map(|embedding| betterlite_semantic::cosine_similarity(query_embedding, embedding))
        .collect())
}

fn embedding_parameters(
    request: &EmbeddingScoringRequest<'_>,
    cache_present: bool,
) -> Map<String, Value> {
    let texts = placeholders(message_texts(request.query, request.messages));
    let mut parameters: Map<String, Value> = Map::new();
    parameters.insert("model".to_owned(), Value::String(request.model.to_owned()));
    parameters.insert(
        "input".to_owned(),
        Value::Array(texts.into_iter().map(Value::String).collect()),
    );
    parameters.insert("caching".to_owned(), Value::Bool(cache_present));
    if let Some(overrides) = request.embedding_model_params {
        for (key, value) in overrides {
            parameters.insert(key.clone(), value.clone());
        }
    }
    parameters
}

fn message_texts(query: &str, messages: &[Value]) -> Vec<String> {
    let mut texts: Vec<String> = Vec::with_capacity(messages.len().saturating_add(1));
    texts.push(truncate_text(query));
    for message in messages {
        let content = message
            .get("content")
            .map(content_to_text)
            .unwrap_or_default();
        texts.push(truncate_text(&content));
    }
    texts
}

fn placeholders(texts: Vec<String>) -> Vec<String> {
    let mut processed: Vec<String> = Vec::with_capacity(texts.len());
    for text in texts {
        if text.trim().is_empty() {
            processed.push(EMPTY_TEXT_PLACEHOLDER.to_owned());
        } else {
            processed.push(text);
        }
    }
    processed
}

fn truncate_text(text: &str) -> String {
    if text.chars().count() <= MAX_EMBEDDING_CHARS {
        return text.to_owned();
    }
    let half = MAX_EMBEDDING_CHARS.saturating_div(2);
    let first_half: String = text.chars().take(half).collect();
    let second_half: String = text
        .chars()
        .rev()
        .take(half)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("{first_half}\n...\n{second_half}")
}

mod tests;
