use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::truncate_message::truncate_message;

use super::CompressError;
use super::counting::TokenCounter;

// LiteLLM truncates a single message only once this many tokens of budget are left
pub const MINIMUM_TRUNCATION_TOKENS: usize = 100;

pub struct BudgetSelectionRequest<'request> {
    pub normalized_messages: &'request [Value],
    pub original_messages: &'request [Value],
    pub combined_scores: &'request [f64],
    pub compression_target: usize,
    pub model: &'request str,
    pub initial_kept_indices: &'request [usize],
    pub tool_exchange_spans: &'request [Vec<usize>],
}

pub struct BudgetSelection {
    pub kept_indices: BTreeSet<usize>,
    pub truncated_overrides: BTreeMap<usize, Value>,
}

struct CandidateUnit {
    score: f64,
    indices: Vec<usize>,
    can_truncate: bool,
}

struct SelectionState {
    kept_indices: BTreeSet<usize>,
    current_tokens: usize,
    truncated_overrides: BTreeMap<usize, Value>,
}

enum CandidateOutcome {
    Continue,
    Exhausted,
}

pub async fn select_kept_indices_for_budget<Tokenizer>(
    request: &BudgetSelectionRequest<'_>,
    counter: &Tokenizer,
) -> Result<BudgetSelection, CompressError>
where
    Tokenizer: TokenCounter,
{
    let mut state = SelectionState {
        kept_indices: request.initial_kept_indices.iter().copied().collect(),
        current_tokens: 0,
        truncated_overrides: BTreeMap::new(),
    };
    let candidates = candidate_units(request, &state.kept_indices);
    let initially_kept: Vec<usize> = state.kept_indices.iter().copied().collect();
    state.current_tokens = count_indices_tokens(request, counter, &initially_kept).await?;
    for candidate in &candidates {
        let outcome = apply_candidate(request, counter, candidate, &mut state).await?;
        if matches!(outcome, CandidateOutcome::Exhausted) {
            break;
        }
    }
    Ok(BudgetSelection {
        kept_indices: state.kept_indices,
        truncated_overrides: state.truncated_overrides,
    })
}

async fn apply_candidate<Tokenizer>(
    request: &BudgetSelectionRequest<'_>,
    counter: &Tokenizer,
    candidate: &CandidateUnit,
    state: &mut SelectionState,
) -> Result<CandidateOutcome, CompressError>
where
    Tokenizer: TokenCounter,
{
    if candidate
        .indices
        .iter()
        .any(|index| state.kept_indices.contains(index))
    {
        return Ok(CandidateOutcome::Continue);
    }
    let message_tokens = count_indices_tokens(request, counter, &candidate.indices).await?;
    let remaining = request
        .compression_target
        .saturating_sub(state.current_tokens);
    if remaining == 0 {
        return Ok(CandidateOutcome::Exhausted);
    }
    if state.current_tokens.saturating_add(message_tokens) <= request.compression_target {
        state.kept_indices.extend(candidate.indices.iter().copied());
        state.current_tokens = state.current_tokens.saturating_add(message_tokens);
        return Ok(CandidateOutcome::Continue);
    }
    if candidate.can_truncate
        && candidate.indices.len() == 1
        && remaining >= MINIMUM_TRUNCATION_TOKENS
    {
        truncate_into_budget(request, counter, candidate, remaining, state).await?;
    }
    Ok(CandidateOutcome::Continue)
}

async fn truncate_into_budget<Tokenizer>(
    request: &BudgetSelectionRequest<'_>,
    counter: &Tokenizer,
    candidate: &CandidateUnit,
    remaining: usize,
    state: &mut SelectionState,
) -> Result<(), CompressError>
where
    Tokenizer: TokenCounter,
{
    let Some(index) = candidate.indices.first().copied() else {
        return Ok(());
    };
    let Some(original) = request.original_messages.get(index) else {
        return Ok(());
    };
    let truncated = truncate_message(original, remaining);
    let text = truncated
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let truncated_tokens = counter
        .count_text(request.model, text)
        .await
        .map_err(CompressError::Counting)?;
    state.truncated_overrides.insert(index, truncated);
    state.kept_indices.insert(index);
    state.current_tokens = state.current_tokens.saturating_add(truncated_tokens);
    Ok(())
}

fn candidate_units(
    request: &BudgetSelectionRequest<'_>,
    kept_indices: &BTreeSet<usize>,
) -> Vec<CandidateUnit> {
    let mut candidates = single_message_units(request, kept_indices);
    candidates.extend(span_units(request, kept_indices));
    candidates.sort_by(|left, right| right.score.total_cmp(&left.score));
    candidates
}

fn single_message_units(
    request: &BudgetSelectionRequest<'_>,
    kept_indices: &BTreeSet<usize>,
) -> Vec<CandidateUnit> {
    let span_indices: BTreeSet<usize> = request
        .tool_exchange_spans
        .iter()
        .flatten()
        .copied()
        .collect();
    let mut candidates: Vec<CandidateUnit> = Vec::new();
    for index in 0..request.normalized_messages.len() {
        if span_indices.contains(&index) || kept_indices.contains(&index) {
            continue;
        }
        candidates.push(CandidateUnit {
            score: request.combined_scores.get(index).copied().unwrap_or(0.0),
            indices: vec![index],
            can_truncate: true,
        });
    }
    candidates
}

fn span_units(
    request: &BudgetSelectionRequest<'_>,
    kept_indices: &BTreeSet<usize>,
) -> Vec<CandidateUnit> {
    let mut candidates: Vec<CandidateUnit> = Vec::new();
    for span in request.tool_exchange_spans {
        let mut indices = span.clone();
        indices.sort_unstable();
        if indices.iter().any(|index| kept_indices.contains(index)) {
            continue;
        }
        let score = indices
            .iter()
            .filter_map(|index| request.combined_scores.get(*index).copied())
            .reduce(f64::max)
            .unwrap_or(0.0);
        candidates.push(CandidateUnit {
            score,
            indices,
            can_truncate: false,
        });
    }
    candidates
}

async fn count_indices_tokens<Tokenizer>(
    request: &BudgetSelectionRequest<'_>,
    counter: &Tokenizer,
    indices: &[usize],
) -> Result<usize, CompressError>
where
    Tokenizer: TokenCounter,
{
    let mut total = 0_usize;
    for index in indices {
        total = total.saturating_add(count_message_text(request, counter, *index).await?);
    }
    Ok(total)
}

async fn count_message_text<Tokenizer>(
    request: &BudgetSelectionRequest<'_>,
    counter: &Tokenizer,
    index: usize,
) -> Result<usize, CompressError>
where
    Tokenizer: TokenCounter,
{
    let text = request
        .normalized_messages
        .get(index)
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    counter
        .count_text(request.model, text)
        .await
        .map_err(CompressError::Counting)
}
