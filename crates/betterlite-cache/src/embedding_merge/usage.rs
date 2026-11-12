use betterlite_models::decisions::Extra;

use super::details::merge_prompt_tokens_details;
use super::error::MergeError;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: Option<Extra>,
}

// LiteLLM's combine_usage: the token counts add and the prompt token details merge field by field
pub fn combine_usage(first: &Usage, second: &Usage) -> Result<Usage, MergeError> {
    Ok(Usage {
        prompt_tokens: first
            .prompt_tokens
            .checked_add(second.prompt_tokens)
            .ok_or(MergeError::Overflow {
                field: "prompt_tokens",
            })?,
        completion_tokens: first
            .completion_tokens
            .checked_add(second.completion_tokens)
            .ok_or(MergeError::Overflow {
                field: "completion_tokens",
            })?,
        total_tokens: first.total_tokens.checked_add(second.total_tokens).ok_or(
            MergeError::Overflow {
                field: "total_tokens",
            },
        )?,
        prompt_tokens_details: merge_prompt_tokens_details(
            first.prompt_tokens_details.as_ref(),
            second.prompt_tokens_details.as_ref(),
        )?,
    })
}
