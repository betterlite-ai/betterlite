use super::{XaiUsage, XaiUsageError, xai_reported_cost_in_usd};

// xAI reports reasoning tokens beside completion_tokens but inside total_tokens; OpenAI folds them into completion
pub fn fold_reasoning(usage: &mut XaiUsage) -> Result<(), XaiUsageError> {
    let tokens = &mut usage.tokens;
    let reasoning_tokens = tokens
        .completion_details
        .and_then(|details| details.reasoning_tokens)
        .unwrap_or(0);
    if reasoning_tokens <= 0 {
        return Ok(());
    }
    let without_reasoning = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    let with_reasoning = without_reasoning
        .checked_add(reasoning_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    if tokens.total_tokens != without_reasoning && tokens.total_tokens == with_reasoning {
        tokens.completion_tokens = tokens
            .completion_tokens
            .checked_add(reasoning_tokens)
            .ok_or(XaiUsageError::Overflow)?;
    }
    Ok(())
}

pub fn normalize_totals(usage: &mut XaiUsage) -> Result<(), XaiUsageError> {
    let tokens = &mut usage.tokens;
    let expected_total = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    tokens.total_tokens = tokens.total_tokens.max(expected_total);
    Ok(())
}

#[must_use]
pub fn usage_restated_from_xai_ticks(usage: &XaiUsage) -> Option<XaiUsage> {
    let reported_cost = xai_reported_cost_in_usd(usage.cost_in_usd_ticks.as_ref()?)?;
    let mut restated = usage.clone();
    restated.cost = Some(reported_cost);
    Some(restated)
}
