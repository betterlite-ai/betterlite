use std::ops::Div;

use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

use crate::embedding::response::EmbeddingResponse;

#[derive(Clone, Copy, Eq, PartialEq)]
enum DetailKind {
    Integer,
    Float,
    Value,
}

// The fields PromptTokensDetailsWrapper declares, so the split follows the type pydantic coerced the value to.
fn declared_kind(name: &str) -> DetailKind {
    match name {
        "audio_tokens"
        | "cached_tokens"
        | "text_tokens"
        | "image_tokens"
        | "video_tokens"
        | "web_search_requests"
        | "google_maps_grounding_requests"
        | "tool_use_tokens"
        | "character_count"
        | "image_count"
        | "query_count"
        | "cache_write_tokens"
        | "cache_creation_tokens" => DetailKind::Integer,
        "video_length_seconds" | "audio_length_seconds" => DetailKind::Float,
        _ => DetailKind::Value,
    }
}

// LiteLLM's _get_per_item_prompt_tokens: a single item keeps the total, a batch splits it with the remainder on the earliest indices.
#[must_use]
pub fn per_item_prompt_tokens(result: &EmbeddingResponse, idx: usize) -> Option<i64> {
    let total = result.usage.as_ref()?.prompt_tokens?;
    let items = i128::try_from(result.data.len()).ok()?;
    let index = i128::try_from(idx).ok()?;
    let wide_total = i128::from(total);
    if items <= 1 {
        return Some(total);
    }
    let quotient = wide_total.checked_div_euclid(items)?;
    let remainder = wide_total.checked_rem_euclid(items)?;
    let share = if index < remainder {
        quotient.checked_add(1)?
    } else {
        quotient
    };
    i64::try_from(share).ok()
}

// LiteLLM's _get_per_item_prompt_tokens_details: null fields are dropped, and a batch splits each field the way its declared type divides.
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
    let mirrored = mirror_cache_tokens(present);
    if result.data.len() <= 1 {
        return Some(mirrored);
    }
    let per_item = mirrored
        .into_iter()
        .map(|(name, value)| {
            let kind = declared_kind(&name);
            Some((
                name,
                distributed_detail(kind, value, result.data.len(), idx)?,
            ))
        })
        .collect::<Option<Extra>>()?;
    Some(per_item)
}

// PromptTokensDetailsWrapper keeps cache_write_tokens and cache_creation_tokens in sync, and cache_write_tokens wins when both arrive.
fn mirror_cache_tokens(mut details: Extra) -> Extra {
    let write = details.get("cache_write_tokens").cloned();
    let creation = details.get("cache_creation_tokens").cloned();
    let nested = details
        .get("cache_creation_input_tokens")
        .filter(|value| matches!(value, Json::Number(number) if number.as_i128().is_some()))
        .cloned();
    let Some(mirrored) = write.or(creation).or(nested) else {
        return details;
    };
    details.insert("cache_write_tokens".to_owned(), mirrored.clone());
    details.insert("cache_creation_tokens".to_owned(), mirrored);
    details
}

fn distributed_detail(kind: DetailKind, value: Json, count: usize, idx: usize) -> Option<Json> {
    let items = i128::try_from(count).ok()?;
    let index = i128::try_from(idx).ok()?;
    match kind {
        DetailKind::Integer => Some(distributed_integer(value, items, index)),
        DetailKind::Float => Some(distributed_float(value, items)),
        DetailKind::Value => distributed_value(value, items, index),
    }
}

fn exact_integer(number: &Number) -> Option<i128> {
    if let Some(value) = number.as_i128() {
        return Some(value);
    }
    let value = number.as_f64()?;
    if !value.is_finite() || value.trunc().to_bits() != value.to_bits() {
        return None;
    }
    value.to_string().parse::<i128>().ok()
}

fn distributed_integer(value: Json, items: i128, index: i128) -> Json {
    let Json::Number(number) = &value else {
        return value;
    };
    let Some(total) = exact_integer(number) else {
        return value;
    };
    let Some(share) = integer_share(total, items, index) else {
        return value;
    };
    Number::from_i128(share).map_or(value, Json::Number)
}

fn integer_share(total: i128, items: i128, index: i128) -> Option<i128> {
    let quotient = total.checked_div_euclid(items)?;
    let remainder = total.checked_rem_euclid(items)?;
    if index < remainder {
        quotient.checked_add(1)
    } else {
        Some(quotient)
    }
}

fn distributed_float(value: Json, items: i128) -> Json {
    if let (Json::Number(number), Ok(count)) = (&value, u32::try_from(items))
        && let Some(total) = number.as_f64()
        && let Some(scaled) = Number::from_f64(Div::div(total, f64::from(count)))
    {
        return Json::Number(scaled);
    }
    value
}

fn distributed_value(value: Json, items: i128, index: i128) -> Option<Json> {
    let Json::Number(number) = value else {
        return Some(value);
    };
    if let Some(total) = number.as_i128() {
        return integer_share(total, items, index)
            .and_then(Number::from_i128)
            .map(Json::Number);
    }
    let total = number.as_f64()?;
    let divisor = f64::from(u32::try_from(items).ok()?);
    Number::from_f64(Div::div(total, divisor)).map(Json::Number)
}
