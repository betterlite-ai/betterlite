use std::slice::from_ref;

use betterlite_models::decisions::Json;

use super::super::json;
use super::shorten::shorten_message_to_fit_limit;
use super::{MINIMUM_AVAILABLE_TOKENS, TrimError, Trimmer, system_message_event};

#[derive(Debug, Clone, Copy)]
pub struct TokenBudget {
    pub available: i64,
    pub max: i64,
}

pub async fn process_system_message(
    trimmer: &Trimmer<'_>,
    system_message: &str,
    max_tokens: i64,
    model: &str,
) -> Result<(Json, i64), TrimError> {
    let mut event = system_message_event(system_message);
    let mut tokens = trimmer.get_token_count(from_ref(&event), model).await?;
    if tokens > max_tokens {
        event = shorten_message_to_fit_limit(trimmer, &event, max_tokens, model, false).await?;
        tokens = trimmer.get_token_count(from_ref(&event), model).await?;
    }
    let remaining = max_tokens
        .checked_sub(tokens)
        .ok_or(TrimError::CountOverflow)?;
    Ok((event, remaining))
}

pub async fn process_messages(
    trimmer: &Trimmer<'_>,
    messages: &[Json],
    max_tokens: i64,
    model: &str,
) -> Result<Vec<Json>, TrimError> {
    let mut final_messages: Vec<Json> = Vec::new();
    for message in messages.iter().rev() {
        let used_tokens = trimmer.get_token_count(&final_messages, model).await?;
        let available = max_tokens
            .checked_sub(used_tokens)
            .ok_or(TrimError::CountOverflow)?;
        if available <= MINIMUM_AVAILABLE_TOKENS {
            break;
        }
        let budget = TokenBudget {
            available,
            max: max_tokens,
        };
        final_messages =
            attempt_message_addition(trimmer, &final_messages, message, budget, model).await?;
    }
    Ok(final_messages)
}

pub async fn attempt_message_addition(
    trimmer: &Trimmer<'_>,
    final_messages: &[Json],
    message: &Json,
    budget: TokenBudget,
    model: &str,
) -> Result<Vec<Json>, TrimError> {
    let mut temp_messages = Vec::with_capacity(final_messages.len().saturating_add(1));
    temp_messages.push(message.clone());
    temp_messages.extend(final_messages.iter().cloned());
    let temp_tokens = trimmer.get_token_count(&temp_messages, model).await?;
    if temp_tokens <= budget.max {
        return Ok(temp_messages);
    }
    let has_function_call = json::object(message)
        .ok_or(TrimError::InvalidMessage)?
        .contains_key("function_call");
    if has_function_call {
        return Ok(final_messages.to_vec());
    }
    let updated =
        shorten_message_to_fit_limit(trimmer, message, budget.available, model, false).await?;
    if !can_add_message(trimmer, &updated, final_messages, budget.max, model).await? {
        return Ok(final_messages.to_vec());
    }
    let mut combined = Vec::with_capacity(final_messages.len().saturating_add(1));
    combined.push(updated);
    combined.extend(final_messages.iter().cloned());
    Ok(combined)
}

pub async fn can_add_message(
    trimmer: &Trimmer<'_>,
    message: &Json,
    messages: &[Json],
    max_tokens: i64,
    model: &str,
) -> Result<bool, TrimError> {
    let mut combined = Vec::with_capacity(messages.len().saturating_add(1));
    combined.extend(messages.iter().cloned());
    combined.push(message.clone());
    let tokens = trimmer.get_token_count(&combined, model).await?;
    Ok(tokens <= max_tokens)
}
