use std::str::from_utf8;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::{Content, Message};
use jiff::Timestamp;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::cache_prefix::marking::is_cached_message;

// 9999-12-31T23:59:59Z, the last expiry Google accepts
const LAST_EXPIRY_SECOND: i64 = 253_402_300_799;
const TTL_DECIMAL_PLACES: u32 = 9;
const CACHE_CONTROL: &str = "cache_control";
const TYPE: &str = "type";
const TTL: &str = "ttl";
const EPHEMERAL: &str = "ephemeral";

#[must_use]
pub fn parse_cache_ttl(ttl: &[u8], now: Timestamp) -> Option<String> {
    let (number, multiplier) = split_unit(ttl)?;
    let value: Decimal = number.parse().ok()?;
    let seconds = value
        .checked_mul(Decimal::from(multiplier))?
        .round_dp_with_strategy(TTL_DECIMAL_PLACES, RoundingStrategy::MidpointNearestEven);
    let longest = longest_ttl(now)?;
    if seconds <= Decimal::ZERO || seconds > longest {
        return None;
    }
    Some(format!("{}s", seconds.normalize()))
}

pub fn extract_cached_ttl(
    messages: &[Message],
    disable_transform: bool,
    now: Timestamp,
) -> Option<String> {
    for message in messages {
        if !is_cached_message(message, disable_transform) {
            continue;
        }
        let Some(Content::Parts(items)) = &message.content else {
            continue;
        };
        for item in items {
            let Some(cache_control) = item.fields.get(CACHE_CONTROL).and_then(Json::as_object)
            else {
                continue;
            };
            if cache_control.get(TYPE).and_then(Json::as_str) != Some(EPHEMERAL) {
                continue;
            }
            let Some(Json::Text(ttl)) = cache_control.get(TTL) else {
                continue;
            };
            if let Some(normalized) = parse_cache_ttl(ttl.as_bytes(), now) {
                return Some(normalized);
            }
        }
    }
    None
}

fn split_unit(ttl: &[u8]) -> Option<(String, u32)> {
    let (&unit, body) = ttl.split_last()?;
    let multiplier = match unit {
        b's' => 1,
        b'm' => 60,
        b'h' => 3_600,
        _ => return None,
    };
    let text = from_utf8(body).ok()?;
    if !is_decimal_number(text) {
        return None;
    }
    let number = if text.starts_with('.') {
        format!("0{text}")
    } else {
        text.to_owned()
    };
    Some((number, multiplier))
}

fn is_decimal_number(text: &str) -> bool {
    let mut digits = 0_usize;
    let mut dots = 0_usize;
    for character in text.chars() {
        match character {
            '0'..='9' => digits = digits.saturating_add(1),
            '.' => {
                dots = dots.saturating_add(1);
                if dots > 1 {
                    return false;
                }
            }
            _ => return false,
        }
    }
    if digits == 0 {
        return false;
    }
    match text.split_once('.') {
        None => true,
        Some((_, fraction)) => !fraction.is_empty(),
    }
}

fn longest_ttl(now: Timestamp) -> Option<Decimal> {
    let whole = LAST_EXPIRY_SECOND.checked_sub(now.as_second())?;
    let subsecond =
        Decimal::try_new(i64::from(now.subsec_nanosecond()), TTL_DECIMAL_PLACES).ok()?;
    Decimal::from(whole).checked_sub(subsecond)
}
