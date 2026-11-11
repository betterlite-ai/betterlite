use std::ops::Div;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use num_traits::cast::cast;

use super::super::json;
use super::{TrimError, Trimmer};

const GPT_MINIMUM_TRIMMING_BUDGET: i64 = 10;
const TRIMMED_JOINER: &str = "..";

pub async fn shorten_message_to_fit_limit(
    trimmer: &Trimmer<'_>,
    message: &Json,
    tokens_needed: i64,
    model: &str,
    raise_error_on_max_limit: bool,
) -> Result<Json, TrimError> {
    // LiteLLM leaves an OpenAI message alone below this budget because even an empty one costs more
    if model.contains("gpt") && tokens_needed <= GPT_MINIMUM_TRIMMING_BUDGET {
        return Ok(message.clone());
    }
    let mut shortened = message.clone();
    let mut content = content_text(&shortened)?.to_owned();
    let mut attempts = 0_usize;
    while attempts < trimmer.max_attempts {
        let total_tokens = trimmer.get_token_count(from_ref(&shortened), model).await?;
        if total_tokens <= tokens_needed {
            break;
        }
        content = shorten_content(&content, tokens_needed, total_tokens)?;
        set_content(&mut shortened, &content)?;
        attempts = attempts.saturating_add(1);
    }
    if attempts >= trimmer.max_attempts && raise_error_on_max_limit {
        return Err(TrimError::NotShortened { attempts });
    }
    Ok(shortened)
}

fn content_text(message: &Json) -> Result<&str, TrimError> {
    json::object(message)
        .and_then(|fields| fields.get("content"))
        .and_then(json::text)
        .ok_or(TrimError::MissingContent)
}

fn set_content(message: &mut Json, content: &str) -> Result<(), TrimError> {
    let Json::Object(fields) = message else {
        return Err(TrimError::MissingContent);
    };
    fields.insert(String::from("content"), Json::Text(content.to_owned()));
    Ok(())
}

fn shorten_content(
    content: &str,
    tokens_needed: i64,
    total_tokens: i64,
) -> Result<String, TrimError> {
    let needed = cast::<i64, f64>(tokens_needed).ok_or(TrimError::CountOverflow)?;
    let total = cast::<i64, f64>(total_tokens).ok_or(TrimError::CountOverflow)?;
    let characters = cast::<usize, f64>(content.chars().count()).ok_or(TrimError::CountOverflow)?;
    let ratio = Div::div(needed, total);
    let scaled =
        cast::<f64, i64>(characters.mul_add(ratio, 0.0_f64)).ok_or(TrimError::CountOverflow)?;
    let new_length = scaled.saturating_sub(1).max(0);
    let half = usize::try_from(new_length.checked_div(2).ok_or(TrimError::CountOverflow)?)
        .map_err(|_conversion| TrimError::CountOverflow)?;
    let head: String = content.chars().take(half).collect();
    // Python's content[-0:] is the whole string, so a zero half keeps the tail whole and the join grows it
    let tail: String = if half == 0 {
        content.to_owned()
    } else {
        content
            .chars()
            .skip(content.chars().count().saturating_sub(half))
            .collect()
    };
    let mut trimmed = head;
    trimmed.push_str(TRIMMED_JOINER);
    trimmed.push_str(&tail);
    Ok(trimmed)
}
