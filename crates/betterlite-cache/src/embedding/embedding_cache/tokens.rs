use std::ops::Div;

use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use crate::embedding::response::EmbeddingResponse;

// LiteLLM's _get_per_item_prompt_tokens: a single item keeps the total, a batch splits it with the remainder on the earliest indices.
#[must_use]
pub fn per_item_prompt_tokens(result: &EmbeddingResponse, idx: usize) -> Option<i64> {
    let total = result.usage.as_ref()?.prompt_tokens?;
    let count = i64::try_from(result.data.len()).ok()?;
    if count <= 1 {
        return Some(total);
    }
    let items = i128::from(count);
    let index = i128::try_from(idx).ok()?;
    let wide_total = i128::from(total);
    let quotient = wide_total.checked_div_euclid(items)?;
    let remainder = wide_total.checked_rem_euclid(items)?;
    let share = if index < remainder {
        quotient.checked_add(1)?
    } else {
        quotient
    };
    i64::try_from(share).ok()
}

// LiteLLM's _get_per_item_prompt_tokens_details: null fields are dropped, and a batch splits every integer field with the earliest indices taking the remainder.
#[must_use]
pub fn per_item_prompt_tokens_details(result: &EmbeddingResponse, idx: usize) -> Option<Extra> {
    let details = result.usage.as_ref()?.prompt_tokens_details.as_ref()?;
    let present = details
        .iter()
        .filter(|(_, value)| !matches!(value, Json::Null))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Extra>();
    if present.is_empty() {
        return None;
    }
    if result.data.len() <= 1 {
        return Some(present);
    }
    let items = i128::try_from(result.data.len()).ok()?;
    let index = i128::try_from(idx).ok()?;
    let per_item = present
        .into_iter()
        .map(|(name, value)| Some((name, distributed_value(value, items, index)?)))
        .collect::<Option<Extra>>()?;
    Some(per_item)
}

// Python's divmod on the one side and true division on the other; any other json value passes through unchanged.
fn distributed_value(value: Json, items: i128, index: i128) -> Option<Json> {
    let Json::Number(number) = value else {
        return Some(value);
    };
    if let Some(total) = number.as_i128() {
        let quotient = total.checked_div_euclid(items)?;
        let remainder = total.checked_rem_euclid(items)?;
        let share = if index < remainder {
            quotient.checked_add(1)?
        } else {
            quotient
        };
        return Number::from_i128(share).map(Json::Number);
    }
    let total = number.as_f64()?;
    let divisor = f64::from(u32::try_from(items).ok()?);
    Number::from_f64(Div::div(total, divisor)).map(Json::Number)
}
