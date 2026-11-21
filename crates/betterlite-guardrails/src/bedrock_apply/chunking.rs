use betterlite_bedrock_guardrails::bedrock_post::GuardrailTransport;

use super::error::ApplyError;
use super::post::PostCall;
use super::retry::posted_with_retry;
use crate::bedrock_chunking::{ChunkResult, bin_pack, split, uses_grounding};
use crate::bedrock_records::{ContentItem, Usage};

// LiteLLM's _apply_guardrail_content_with_chunking: probe the whole payload first, then shrink it only on demand
pub(super) async fn chunked<T>(
    call: &mut PostCall<'_>,
    content: Vec<ContentItem>,
    transport: &T,
    completed: &mut Vec<Usage>,
) -> Result<Vec<ChunkResult>, ApplyError>
where
    T: GuardrailTransport,
{
    match posted_with_retry(call, &content, transport, completed).await {
        Ok(response) => Ok(vec![ChunkResult {
            response,
            content,
            fragment_group_size: 1,
        }]),
        // a split keeps every qualifier, so a grounded payload stays grounded at every level of the recursion
        Err(error) if !uses_grounding(&content) && error.is_too_large() => {
            Box::pin(recover(call, content, error, transport, completed)).await
        }
        Err(error) => Err(error),
    }
}

async fn recover<T>(
    call: &mut PostCall<'_>,
    content: Vec<ContentItem>,
    error: ApplyError,
    transport: &T,
    completed: &mut Vec<Usage>,
) -> Result<Vec<ChunkResult>, ApplyError>
where
    T: GuardrailTransport,
{
    let budget = usize::try_from(call.guardrail.chunk_budget_chars).unwrap_or(usize::MAX);
    let batches = bin_pack(&content, budget);
    if batches.len() > 1 {
        tracing::warn!(
            "Bedrock Guardrail: ApplyGuardrail rejected {} content item(s) as too large; re-sending as {} batches of at most {} characters",
            content.len(),
            batches.len(),
            budget
        );
        let mut results = Vec::new();
        for batch in batches {
            let mut part = Box::pin(chunked(call, batch, transport, completed)).await?;
            results.append(&mut part);
        }
        return Ok(results);
    }
    let halves = split(&content).ok().flatten();
    let Some((first_half, second_half)) = halves else {
        return Err(error);
    };
    let single_item_split = content.len() == 1;
    tracing::warn!(
        "Bedrock Guardrail: ApplyGuardrail rejected {} content item(s) as too large; splitting into {} + {} and retrying each",
        content.len(),
        first_half.len(),
        second_half.len()
    );
    let mut combined = Box::pin(chunked(call, first_half, transport, completed)).await?;
    let mut rest = Box::pin(chunked(call, second_half, transport, completed)).await?;
    combined.append(&mut rest);
    if single_item_split {
        let leaves = combined.len();
        for result in &mut combined {
            result.fragment_group_size = leaves;
        }
    }
    Ok(combined)
}
