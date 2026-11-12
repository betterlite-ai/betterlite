use std::collections::BTreeMap;

use betterlite_models::call_helpers::finish_reason::FinishReason;
use betterlite_models::usage_sum::Usage;

use crate::gemini_candidates::candidate::{Candidate, PromptFeedback};
use crate::gemini_candidates::choice::{Choice, Delta, Message, Role, StreamChoice};
use crate::gemini_candidates::error::CandidatesError;
use crate::gemini_usage::error::UsageError;
use crate::gemini_usage::metadata::UsageMetadata;

pub const FLAGGED_FINISH_REASONS: [&str; 7] = [
    "SAFETY",
    "RECITATION",
    "BLOCKLIST",
    "PROHIBITED_CONTENT",
    "SPII",
    "IMAGE_SAFETY",
    "IMAGE_PROHIBITED_CONTENT",
];

#[derive(Debug)]
pub struct Blocked {
    pub choice: Choice,
    pub usage: Usage,
}

// both blocked paths copy the three raw counters, so normal usage enrichment never runs
pub fn complete_block(
    feedback: Option<&PromptFeedback>,
    candidates: &[Candidate],
    usage: Option<&UsageMetadata>,
) -> Result<Option<Blocked>, CandidatesError> {
    let prompt_blocked = feedback.is_some_and(|prompt| prompt.block_reason.is_some());
    // only the first candidate decides, a flagged reason on a later one is an ordinary choice
    let flagged = candidates
        .first()
        .and_then(|candidate| candidate.finish_reason.as_deref())
        .is_some_and(|reason| FLAGGED_FINISH_REASONS.contains(&reason));
    if !prompt_blocked && !flagged {
        return Ok(None);
    }
    let counters = usage.ok_or(UsageError::MissingUsageMetadata)?;
    Ok(Some(Blocked {
        choice: Choice {
            index: 0,
            finish_reason: FinishReason::ContentFilter.as_str(),
            message: Message::default(),
            logprobs: None,
            native_finish_reason: None,
        },
        usage: Usage {
            prompt_tokens: counters.prompt_token_count,
            completion_tokens: counters.candidates_token_count,
            total_tokens: counters.total_token_count,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            numeric_extras: BTreeMap::new(),
        },
    }))
}

// the streamed check only looks for the blockReason key, never at the candidates
#[must_use]
pub fn stream_block(feedback: Option<&PromptFeedback>) -> Option<StreamChoice> {
    feedback?.block_reason.as_ref()?;
    Some(StreamChoice {
        index: 0,
        finish_reason: FinishReason::ContentFilter.as_str(),
        delta: Delta {
            role: Some(Role::Assistant),
            content: None,
            reasoning_content: None,
            tool_calls: None,
            function_call: None,
            images: None,
            annotations: None,
            provider_specific_fields: None,
        },
        logprobs: None,
    })
}
