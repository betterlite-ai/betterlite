use betterlite_models::decisions::Json;

pub const MIN_CODE: u16 = 400;
pub const MAX_CODE: u16 = 599;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexGemmaContainerError {
    pub code: u16,
    pub message: String,
}

// LiteLLM's pydantic record ignores unknown keys, requires object="error" and a string message, and coerces the code
#[must_use]
pub fn parse_gemma_container_error(predictions: &Json) -> Option<VertexGemmaContainerError> {
    let Json::Object(fields) = predictions else {
        return None;
    };
    let Json::Text(object) = fields.get("object")? else {
        return None;
    };
    if object != "error" {
        return None;
    }
    let Json::Text(message) = fields.get("message")? else {
        return None;
    };
    let code = match fields.get("code")? {
        // pydantic reads a float the way it reads its shortest round-trip text
        Json::Number(number) => text_code(&number.as_f64()?.to_string()),
        Json::Text(text) => text_code(text),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }?;
    Some(VertexGemmaContainerError {
        code,
        message: message.to_owned(),
    })
}

// pydantic reads the string the way Python's int() does: ASCII digits, single underscores between them, and a zero fraction
fn text_code(text: &str) -> Option<u16> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let unsigned = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('+'))
        .unwrap_or(trimmed);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if whole.starts_with('_') || whole.ends_with('_') || whole.contains("__") {
        return None;
    }
    if let Some(rest) = fraction
        && (rest.is_empty() || !rest.bytes().all(|byte| byte == b'0'))
    {
        return None;
    }
    let mut value: i64 = 0;
    for byte in whole.bytes() {
        if byte == b'_' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        let digit = u32::from(byte).checked_sub(u32::from(b'0'))?;
        value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
    }
    let signed = if negative {
        value.checked_neg()?
    } else {
        value
    };
    u16::try_from(signed)
        .ok()
        .filter(|code| (MIN_CODE..=MAX_CODE).contains(code))
}
