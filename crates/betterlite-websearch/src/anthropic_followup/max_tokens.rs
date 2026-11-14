use betterlite_context::agentic_loop_plan::JsonMap;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::AnthropicFollowupError;

const DEFAULT_MAX_TOKENS: u64 = 1024;
const THINKING_HEADROOM: u64 = 1024;
const MAX_TOKENS_KEY: &str = "max_tokens";
const THINKING_KEY: &str = "thinking";

#[derive(Deserialize)]
struct ThinkingParam {
    #[serde(default)]
    budget_tokens: Option<Box<RawValue>>,
}

pub fn resolve_max_tokens(
    optional_params: &JsonMap,
    kwargs: &JsonMap,
) -> Result<u64, AnthropicFollowupError> {
    let max_tokens = max_tokens(optional_params)?
        .or(max_tokens(kwargs)?)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let Some(budget_tokens) = budget_tokens(optional_params)? else {
        return Ok(max_tokens);
    };
    if !num_traits::cast::<u64, f64>(max_tokens).is_some_and(|max| max <= budget_tokens) {
        return Ok(max_tokens);
    }
    let adjusted = adjusted_max_tokens(budget_tokens)?;
    tracing::debug!(
        "WebSearchInterception: max_tokens={max_tokens} <= thinking.budget_tokens={budget_tokens}, adjusting to {adjusted} to satisfy Anthropic API constraint"
    );
    Ok(adjusted)
}

fn max_tokens(params: &JsonMap) -> Result<Option<u64>, AnthropicFollowupError> {
    let Some(raw) = params.get(MAX_TOKENS_KEY) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(AnthropicFollowupError::InvalidMaxTokens)
}

fn budget_tokens(optional_params: &JsonMap) -> Result<Option<f64>, AnthropicFollowupError> {
    let Some(raw) = optional_params.get(THINKING_KEY) else {
        return Ok(None);
    };
    let thinking: Option<ThinkingParam> =
        serde_json::from_str(raw.get()).map_err(AnthropicFollowupError::InvalidThinking)?;
    let Some(thinking) = thinking else {
        return Ok(None);
    };
    let Some(raw_budget) = thinking.budget_tokens else {
        return Ok(None);
    };
    let budget: Option<f64> = serde_json::from_str(raw_budget.get())
        .map_err(AnthropicFollowupError::InvalidBudgetTokens)?;
    let Some(budget) = budget else {
        return Ok(None);
    };
    if !budget.is_finite() || budget <= 0.0_f64 {
        return Ok(None);
    }
    Ok(Some(budget))
}

fn adjusted_max_tokens(budget_tokens: f64) -> Result<u64, AnthropicFollowupError> {
    let rounded = num_traits::cast::<f64, u64>(budget_tokens.ceil())
        .ok_or(AnthropicFollowupError::BudgetTokensTooLarge)?;
    rounded
        .checked_add(THINKING_HEADROOM)
        .ok_or(AnthropicFollowupError::BudgetTokensTooLarge)
}
