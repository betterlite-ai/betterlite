use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::num::ParseFloatError;

use crate::anthropic_tool_exchange::extract_exchange_spans;
use crate::bm25_scoring::{
    DEFAULT_LENGTH_NORMALIZATION, DEFAULT_TERM_SATURATION, bm25_score_messages,
};
use crate::embedding_scoring::{
    EmbeddingExecutor, EmbeddingScoringError, EmbeddingScoringRequest, embedding_score_messages,
};
use crate::message_normalization::{
    CallType, CompressionError, extract_last_user_message, normalize_messages,
};
use crate::protected_indices::get_protected_indices;
use crate::retrieval_tool::RetrievalToolError;
use crate::score_combination::{DEFAULT_BM25_WEIGHT, combine_scores};

mod assembly;
pub mod budget;
pub mod counting;

use assembly::assemble_result;
use budget::{BudgetSelectionRequest, select_kept_indices_for_budget};
use counting::TokenCounter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionSkippedReason {
    BelowTrigger,
    InvalidAnthropicToolSequence,
}

impl CompressionSkippedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BelowTrigger => "below_trigger",
            Self::InvalidAnthropicToolSequence => "invalid_anthropic_tool_sequence",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompressedResult {
    pub messages: Vec<Value>,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub compression_ratio: f64,
    pub cache: IndexMap<String, String>,
    pub tools: Vec<Value>,
    pub compression_skipped_reason: Option<CompressionSkippedReason>,
}

pub const DEFAULT_COMPRESSION_TRIGGER: usize = 200_000;

#[derive(Clone, Copy, Debug)]
pub struct CompressRequest<'request> {
    pub messages: &'request [Value],
    pub model: &'request str,
    pub call_type: &'request str,
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
}

pub struct EmbeddingScoring<'stage, Executor> {
    pub model: &'stage str,
    pub parameters: Option<&'stage Map<String, Value>>,
    pub cache_present: bool,
    pub executor: &'stage Executor,
}

#[derive(Debug, thiserror::Error)]
pub enum CompressError {
    #[error(transparent)]
    Normalization(#[from] CompressionError),
    #[error("the token count could not be computed")]
    Counting(#[source] CountError),
    #[error("the compression ratio could not be read back from its decimal form")]
    Ratio(#[source] ParseFloatError),
    #[error(transparent)]
    RetrievalTools(#[from] RetrievalToolError),
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingCompressError<ExecutorFailure>
where
    ExecutorFailure: Error + 'static,
{
    #[error(transparent)]
    Compression(#[from] CompressError),
    #[error("relevance scoring with embeddings failed")]
    Embedding(#[source] EmbeddingScoringError<ExecutorFailure>),
}

struct PreparedCompression {
    call_type: CallType,
    normalized_messages: Vec<Value>,
    original_messages: Vec<Value>,
    original_tokens: usize,
    compression_trigger: usize,
    query: String,
}

enum Preparation {
    Skipped(CompressedResult),
    Scored {
        prepared: PreparedCompression,
        bm25_scores: Vec<f64>,
    },
}

pub async fn compress<Tokenizer>(
    request: &CompressRequest<'_>,
    counter: &Tokenizer,
) -> Result<CompressedResult, CompressError>
where
    Tokenizer: TokenCounter,
{
    match prepare(request, counter).await? {
        Preparation::Skipped(result) => Ok(result),
        Preparation::Scored {
            prepared,
            bm25_scores,
        } => finish(prepared, &bm25_scores, request, counter).await,
    }
}

pub async fn compress_with_embeddings<Tokenizer, Executor>(
    request: &CompressRequest<'_>,
    counter: &Tokenizer,
    embedding: &EmbeddingScoring<'_, Executor>,
) -> Result<CompressedResult, EmbeddingCompressError<Executor::Failure>>
where
    Tokenizer: TokenCounter,
    Executor: EmbeddingExecutor,
{
    match prepare(request, counter).await? {
        Preparation::Skipped(result) => Ok(result),
        Preparation::Scored {
            prepared,
            bm25_scores,
        } => {
            let cache: Option<&()> = embedding.cache_present.then_some(&());
            let embedding_scores = embedding_score_messages(
                EmbeddingScoringRequest {
                    query: &prepared.query,
                    messages: &prepared.normalized_messages,
                    model: embedding.model,
                    embedding_model_params: embedding.parameters,
                },
                cache,
                embedding.executor,
            )
            .await
            .map_err(EmbeddingCompressError::Embedding)?;
            let combined_scores =
                combine_scores(&bm25_scores, &embedding_scores, DEFAULT_BM25_WEIGHT);
            let result = finish(prepared, &combined_scores, request, counter).await?;
            Ok(result)
        }
    }
}

async fn prepare<Tokenizer>(
    request: &CompressRequest<'_>,
    counter: &Tokenizer,
) -> Result<Preparation, CompressError>
where
    Tokenizer: TokenCounter,
{
    let (call_type, normalized_messages, original_messages) =
        normalize_messages(request.messages, request.call_type)?;
    let original_tokens = counter
        .count_messages(request.model, &original_messages)
        .await
        .map_err(CompressError::Counting)?;
    if original_tokens <= request.compression_trigger {
        return Ok(Preparation::Skipped(skipped_result(
            original_messages,
            original_tokens,
            CompressionSkippedReason::BelowTrigger,
        )));
    }
    let query = extract_last_user_message(&normalized_messages);
    let bm25_scores = bm25_score_messages(
        &query,
        &normalized_messages,
        DEFAULT_TERM_SATURATION,
        DEFAULT_LENGTH_NORMALIZATION,
    );
    Ok(Preparation::Scored {
        prepared: PreparedCompression {
            call_type,
            normalized_messages,
            original_messages,
            original_tokens,
            compression_trigger: request.compression_trigger,
            query,
        },
        bm25_scores,
    })
}

async fn finish<Tokenizer>(
    prepared: PreparedCompression,
    combined_scores: &[f64],
    request: &CompressRequest<'_>,
    counter: &Tokenizer,
) -> Result<CompressedResult, CompressError>
where
    Tokenizer: TokenCounter,
{
    let (tool_exchange_spans, sequence_error) = if prepared.call_type == CallType::AnthropicMessages
    {
        extract_exchange_spans(&prepared.original_messages)
    } else {
        (Vec::new(), None)
    };
    if sequence_error.is_some() {
        return Ok(skipped_result(
            prepared.original_messages,
            prepared.original_tokens,
            CompressionSkippedReason::InvalidAnthropicToolSequence,
        ));
    }
    let kept_indices = protected_kept_indices(&prepared.original_messages, &tool_exchange_spans);
    let default_target = prepared
        .compression_trigger
        .saturating_mul(7)
        .checked_div(10)
        .unwrap_or_default();
    let compression_target = request.compression_target.unwrap_or(default_target);
    let selection = select_kept_indices_for_budget(
        &BudgetSelectionRequest {
            normalized_messages: &prepared.normalized_messages,
            original_messages: &prepared.original_messages,
            combined_scores,
            compression_target,
            model: request.model,
            initial_kept_indices: &kept_indices,
            tool_exchange_spans: &tool_exchange_spans,
        },
        counter,
    )
    .await?;
    let result = assemble_result(
        &prepared,
        &selection,
        &tool_exchange_spans,
        request.model,
        counter,
    )
    .await?;
    Ok(result)
}

fn skipped_result(
    messages: Vec<Value>,
    original_tokens: usize,
    reason: CompressionSkippedReason,
) -> CompressedResult {
    CompressedResult {
        messages,
        original_tokens,
        compressed_tokens: original_tokens,
        compression_ratio: 0.0,
        cache: IndexMap::new(),
        tools: Vec::new(),
        compression_skipped_reason: Some(reason),
    }
}

fn protected_kept_indices(
    original_messages: &[Value],
    tool_exchange_spans: &[Vec<usize>],
) -> Vec<usize> {
    let mut kept_indices: BTreeSet<usize> = get_protected_indices(original_messages)
        .into_iter()
        .collect();
    for span in tool_exchange_spans {
        if span.iter().any(|index| kept_indices.contains(index)) {
            kept_indices.extend(span.iter().copied());
        }
    }
    kept_indices.into_iter().collect()
}

mod tests;
