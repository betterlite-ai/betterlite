use std::str::FromStr as _;

use indexmap::IndexMap;
use serde_json::Number;

use super::JsonObject;
use crate::decisions::Json;

pub fn update_dictionary(existing_dict: &mut JsonObject, new_dict: &JsonObject) {
    for (key, value) in new_dict {
        match value {
            Json::Null => {}
            Json::Text(text) => {
                existing_dict.insert(key.clone(), convert_stringified_numbers(text));
            }
            Json::Object(nested) => {
                let mut merged = match existing_dict.get(key) {
                    Some(Json::Object(existing_nested)) => existing_nested.clone(),
                    _ => IndexMap::new(),
                };
                merged.extend(
                    nested
                        .iter()
                        .map(|(name, item)| (name.clone(), item.clone())),
                );
                existing_dict.insert(key.clone(), Json::Object(merged));
            }
            Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                existing_dict.insert(key.clone(), value.clone());
            }
        }
    }
}

#[must_use]
pub fn convert_stringified_numbers(text: &str) -> Json {
    let trimmed = text.trim();
    let converted = if text.to_lowercase().contains('e') || text.contains('.') {
        trimmed.parse::<f64>().ok().and_then(Number::from_f64)
    } else {
        whole_number(trimmed)
    };
    // a float that overflows to infinity has no JSON form, so the text stays
    converted.map_or_else(|| Json::Text(text.to_owned()), Json::Number)
}

fn whole_number(text: &str) -> Option<Number> {
    let (sign, digits) = text.strip_prefix('-').map_or_else(
        || ("", text.strip_prefix('+').unwrap_or(text)),
        |rest| ("-", rest),
    );
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Some(Number::from(0_u8));
    }
    Number::from_str(&format!("{sign}{significant}")).ok()
}
