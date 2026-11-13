use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

#[must_use]
pub fn map_gpt_image_size(size: &Json) -> Json {
    let Json::Text(text) = size else {
        return size.clone();
    };
    if text == "auto" {
        return size.clone();
    }
    let Some((width, height)) = gpt_image_dimensions(text) else {
        return size.clone();
    };
    let mut mapped = IndexMap::new();
    mapped.insert("width".to_owned(), Json::Number(Number::from(width)));
    mapped.insert("height".to_owned(), Json::Number(Number::from(height)));
    Json::Object(mapped)
}

fn gpt_image_dimensions(text: &str) -> Option<(i64, i64)> {
    let lowered = text.to_lowercase();
    let mut parts = lowered.split('x');
    let width = dimension_int(parts.next()?)?;
    let height = dimension_int(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((width, height))
}

// Python's int() reads a sign, digits and single underscores between them; every other form stays a string
fn dimension_int(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, digits) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, trimmed.get(1..)?),
        Some(b'+') => (false, trimmed.get(1..)?),
        _ => (false, trimmed),
    };
    let mut value: i64 = 0;
    let mut after_digit = false;
    for byte in digits.bytes() {
        match byte {
            b'_' if after_digit => after_digit = false,
            b'0'..=b'9' => {
                let digit = i64::from(byte).checked_sub(i64::from(b'0'))?;
                value = value.checked_mul(10)?.checked_add(digit)?;
                after_digit = true;
            }
            _ => return None,
        }
    }
    if !after_digit {
        return None;
    }
    if negative {
        value.checked_neg()
    } else {
        Some(value)
    }
}
