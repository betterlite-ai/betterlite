use std::future::Future;

use betterlite_models::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use betterlite_models::decisions::Json;
use futures_util::future::join_all;
use secrecy::SecretString;

use crate::cache_plan::wire::{Block, Content, PlanRequest};
use crate::cache_plan::{PromptCachePlan, UnsupportedCachePlan};
use crate::cache_prediction_targets::counting::TokenCounter;

pub trait ThinkingCacheSupport: Sync {
    fn preserves_thinking_cache(&self, model: &str) -> impl Future<Output = bool> + Send;
}

pub async fn count_cache_plan<Counter, Support>(
    model: &str,
    api_key: &SecretString,
    plan: &PromptCachePlan,
    counter: &Counter,
    thinking: &Support,
) -> Result<CountedPromptCachePlan, UnsupportedCachePlan>
where
    Counter: TokenCounter,
    Support: ThinkingCacheSupport,
{
    let Ok(body) = serde_json::from_str::<Json>(plan.full_body.get()) else {
        return Err(UnsupportedCachePlan::UnsupportedPromptShape);
    };
    let Ok(request) = serde_json::from_str::<PlanRequest>(plan.full_body.get()) else {
        return Err(UnsupportedCachePlan::UnsupportedPromptShape);
    };
    if has_thinking_block(&request) && !thinking.preserves_thinking_cache(model).await {
        return Err(UnsupportedCachePlan::UnsupportedThinkingCache);
    }
    let Some(total) = counter.count(model, api_key, &body).await else {
        return Err(UnsupportedCachePlan::TokenCountUnavailable);
    };
    let known = count_prefixes(model, api_key, plan, counter).await?;
    if total < 0 || known.iter().any(|count| *count < 0) {
        return Err(UnsupportedCachePlan::InconsistentPrefixTokenCount);
    }
    let rising = known
        .iter()
        .zip(known.iter().skip(1))
        .any(|(first, second)| first > second);
    if rising || known.last().is_some_and(|last| *last > total) {
        return Err(UnsupportedCachePlan::InconsistentPrefixTokenCount);
    }
    Ok(CountedPromptCachePlan {
        total_tokens: total,
        breakpoints: counted_breakpoints(plan, known),
    })
}

async fn count_prefixes<Counter>(
    model: &str,
    api_key: &SecretString,
    plan: &PromptCachePlan,
    counter: &Counter,
) -> Result<Vec<i64>, UnsupportedCachePlan>
where
    Counter: TokenCounter,
{
    let mut prefixes = Vec::with_capacity(plan.breakpoints.len());
    for marker in &plan.breakpoints {
        let Ok(prefix) = serde_json::from_str::<Json>(marker.prefix_body.get()) else {
            return Err(UnsupportedCachePlan::UnsupportedPromptShape);
        };
        prefixes.push(prefix);
    }
    let counts = join_all(
        prefixes
            .iter()
            .map(|prefix| counter.count(model, api_key, prefix)),
    )
    .await;
    let mut known = Vec::with_capacity(counts.len());
    for count in counts {
        let Some(tokens) = count else {
            return Err(UnsupportedCachePlan::TokenCountUnavailable);
        };
        known.push(tokens);
    }
    Ok(known)
}

fn counted_breakpoints(plan: &PromptCachePlan, counts: Vec<i64>) -> Vec<CountedBreakpoint> {
    plan.breakpoints
        .iter()
        .zip(counts)
        .map(|(marker, prefix_tokens)| CountedBreakpoint {
            fingerprint: marker.fingerprint.clone(),
            ttl_seconds: i64::from(marker.ttl_seconds),
            prefix_tokens,
            lookback_fingerprints: marker.lookback_fingerprints.clone(),
            content_fingerprint: marker.content_fingerprint.clone(),
            lookback_content_fingerprints: marker.lookback_content_fingerprints.clone(),
        })
        .collect()
}

// LiteLLM reads the request positions so a thinking block anywhere in the prompt gates the count
fn has_thinking_block(request: &PlanRequest) -> bool {
    request.messages.iter().any(|message| {
        matches!(
            &message.content,
            Content::Blocks(blocks)
                if blocks
                    .iter()
                    .any(|block| matches!(block, Block::Thinking(_)))
        )
    })
}
