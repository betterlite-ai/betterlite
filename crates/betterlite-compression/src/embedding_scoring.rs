use crate::message_normalization::content_to_text;
use async_trait::async_trait;
use serde_json::{Map, Value};
use thiserror::Error;

const MAX_EMBEDDING_CHARS: usize = 30_000;
const EMPTY_TEXT_PLACEHOLDER: &str = "empty";

#[derive(Clone, Copy)]
pub struct EmbeddingCall<'call> {
    pub model: &'call str,
    pub input: &'call [String],
    pub caching: bool,
    pub embedding_model_params: Option<&'call Map<String, Value>>,
}

#[async_trait]
pub trait EmbeddingExecutor: Send + Sync {
    type Failure: std::error::Error + Send + Sync + 'static;

    async fn embed(&self, call: EmbeddingCall<'_>) -> Result<Vec<Vec<f64>>, Self::Failure>;
}

#[derive(Debug, Error)]
pub enum EmbeddingScoringError<ExecutorFailure>
where
    ExecutorFailure: std::error::Error + 'static,
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
    let mut message_texts: Vec<String> =
        Vec::with_capacity(request.messages.len().saturating_add(1));
    message_texts.push(truncate_text(request.query));
    for message in request.messages {
        let content = message
            .get("content")
            .map(content_to_text)
            .unwrap_or_default();
        message_texts.push(truncate_text(&content));
    }
    let mut processed: Vec<String> = Vec::with_capacity(message_texts.len());
    for text in message_texts {
        if text.trim().is_empty() {
            processed.push(EMPTY_TEXT_PLACEHOLDER.to_owned());
        } else {
            processed.push(text);
        }
    }
    let embeddings = executor
        .embed(EmbeddingCall {
            model: request.model,
            input: processed.as_slice(),
            caching: cache.is_some(),
            embedding_model_params: request.embedding_model_params,
        })
        .await
        .map_err(EmbeddingScoringError::Executor)?;
    let query_embedding = embeddings
        .first()
        .ok_or(EmbeddingScoringError::EmptyResponse)?;
    Ok(embeddings
        .iter()
        .skip(1)
        .map(|embedding| cosine_similarity(query_embedding, embedding))
        .collect())
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

fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    let dot_product =
        left.iter()
            .zip(right)
            .fold(0.0_f64, |accumulated, (left_component, right_component)| {
                accumulated.mul_add(1.0, left_component.mul_add(*right_component, 0.0))
            });
    let left_norm = left
        .iter()
        .fold(0.0_f64, |accumulated, component| {
            accumulated.mul_add(1.0, component.mul_add(*component, 0.0))
        })
        .sqrt();
    let right_norm = right
        .iter()
        .fold(0.0_f64, |accumulated, component| {
            accumulated.mul_add(1.0, component.mul_add(*component, 0.0))
        })
        .sqrt();
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    dot_product.mul_add(left_norm.mul_add(right_norm, 0.0).recip(), 0.0)
}

mod tests;
