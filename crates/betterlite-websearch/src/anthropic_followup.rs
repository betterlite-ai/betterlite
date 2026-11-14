pub mod followup_kwargs;
pub mod max_tokens;
pub mod patch;
pub mod plan;

mod tests;

use thiserror::Error;

use crate::call_transformation::TransformationError;
use crate::chat_responses_followup::error::FollowupError;

#[derive(Debug, Error)]
pub enum AnthropicFollowupError {
    #[error("cannot build the follow-up plan with the chat or responses builder")]
    Followup(#[from] FollowupError),
    #[error("cannot transform the search results for the follow-up request")]
    Transformation(#[from] TransformationError),
    #[error("the search results use the openai format, not the anthropic one")]
    UnexpectedResultFormat,
    #[error("cannot encode a follow-up request or plan value")]
    Json(#[from] serde_json::Error),
    #[error("max_tokens must be a non-negative integer")]
    InvalidMaxTokens(#[source] serde_json::Error),
    #[error("thinking must be an object")]
    InvalidThinking(#[source] serde_json::Error),
    #[error("thinking.budget_tokens must be a number")]
    InvalidBudgetTokens(#[source] serde_json::Error),
    #[error("thinking.budget_tokens is too large for a follow-up max_tokens")]
    BudgetTokensTooLarge,
    #[error("agentic_loop_params must be an object")]
    InvalidAgenticLoopParams(#[source] serde_json::Error),
    #[error("_websearch_interception_emit_native_blocks must be a boolean")]
    InvalidNativeBlocksFlag(#[source] serde_json::Error),
}
