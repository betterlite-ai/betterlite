use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::error::MaiImageError;

// The object to read, the key in it and the field name an error reports
type TokenSource<'fields> = (&'fields Object, &'static str, &'static str);

struct AliasCounts {
    text: i64,
    image: i64,
    output: i64,
}

// A count is absent, null or a whole number; anything else names the field
fn token_field(source: TokenSource<'_>) -> Result<Option<i64>, MaiImageError> {
    let (fields, key, field) = source;
    match fields.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or(MaiImageError::FieldType { field }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(MaiImageError::FieldType { field })
        }
    }
}

// LiteLLM reads the aliases in order and stops at the first one that is not None, so a later alias is never inspected
fn first_token(sources: &[TokenSource<'_>]) -> Result<i64, MaiImageError> {
    for source in sources {
        if let Some(tokens) = token_field(*source)? {
            return Ok(tokens);
        }
    }
    Ok(0)
}

fn alias_counts(usage: &Object) -> Result<AliasCounts, MaiImageError> {
    let no_details = Object::new();
    let details = usage
        .get("input_tokens_details")
        .and_then(Json::as_object)
        .unwrap_or(&no_details);
    Ok(AliasCounts {
        text: first_token(&[
            (usage, "num_input_text_tokens", "num_input_text_tokens"),
            (details, "text_tokens", "input_tokens_details.text_tokens"),
            (usage, "input_tokens", "input_tokens"),
        ])?,
        image: first_token(&[
            (usage, "num_input_image_tokens", "num_input_image_tokens"),
            (details, "image_tokens", "input_tokens_details.image_tokens"),
        ])?,
        output: first_token(&[
            (usage, "output_tokens", "output_tokens"),
            (usage, "num_output_tokens", "num_output_tokens"),
            (usage, "output_image_tokens", "output_image_tokens"),
        ])?,
    })
}

// An explicit input or total, zero included, wins over the sum of what was counted
fn explicit_or_sum(
    usage: &Object,
    key: &'static str,
    parts: (i64, i64),
) -> Result<i64, MaiImageError> {
    let explicit = token_field((usage, key, key))?;
    explicit.map_or_else(
        || {
            parts
                .0
                .checked_add(parts.1)
                .ok_or(MaiImageError::TokenOverflow { field: key })
        },
        Ok,
    )
}

pub fn normalize_mai_image_usage(reported: Option<&Object>) -> Result<Object, MaiImageError> {
    // None gives the same zeroed shape as an empty usage object
    let empty = Object::new();
    let usage = reported.unwrap_or(&empty);
    let counts = alias_counts(usage)?;
    let input_tokens = explicit_or_sum(usage, "input_tokens", (counts.text, counts.image))?;
    let total_tokens = explicit_or_sum(usage, "total_tokens", (input_tokens, counts.output))?;
    let details = Object::from_iter([
        (
            "image_tokens".to_owned(),
            Json::Number(Number::from(counts.image)),
        ),
        (
            "text_tokens".to_owned(),
            Json::Number(Number::from(counts.text)),
        ),
    ]);
    let mut normalized = usage.clone();
    let numbers = [
        ("input_tokens", input_tokens),
        ("output_tokens", counts.output),
        ("total_tokens", total_tokens),
    ];
    for (key, value) in numbers {
        normalized.insert(key.to_owned(), Json::Number(Number::from(value)));
    }
    normalized.insert("input_tokens_details".to_owned(), Json::Object(details));
    Ok(normalized)
}
