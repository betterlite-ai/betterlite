use super::{XaiUsage, XaiUsageError, xai_reported_cost_in_usd};

// xAI reports reasoning tokens beside completion_tokens but inside total_tokens; OpenAI folds them into completion
pub fn fold_reasoning(usage: &mut XaiUsage) -> Result<(), XaiUsageError> {
    let tokens = &mut usage.tokens;
    let reasoning = tokens
        .completion_details
        .and_then(|details| details.reasoning_tokens)
        .unwrap_or(0);
    if reasoning <= 0 {
        return Ok(());
    }
    let visible = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    let included = visible
        .checked_add(reasoning)
        .ok_or(XaiUsageError::Overflow)?;
    if tokens.total_tokens == included && tokens.total_tokens != visible {
        tokens.completion_tokens = tokens
            .completion_tokens
            .checked_add(reasoning)
            .ok_or(XaiUsageError::Overflow)?;
    }
    Ok(())
}

pub fn normalize_totals(usage: &mut XaiUsage) -> Result<(), XaiUsageError> {
    let tokens = &mut usage.tokens;
    let expected = tokens
        .prompt_tokens
        .checked_add(tokens.completion_tokens)
        .ok_or(XaiUsageError::Overflow)?;
    tokens.total_tokens = tokens.total_tokens.max(expected);
    Ok(())
}

#[must_use]
pub fn usage_restated_from_xai_ticks(usage: &XaiUsage) -> Option<XaiUsage> {
    let mut restated = usage.clone();
    restated.cost = Some(xai_reported_cost_in_usd(usage.cost_in_usd_ticks.as_ref()?)?);
    Some(restated)
}
