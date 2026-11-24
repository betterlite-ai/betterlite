use std::fmt;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::oauth_token_resolution::ports::TokenResponse;

pub struct GrantBody {
    pub access_token: String,
    pub rotated_refresh: Option<String>,
    pub expires_in: Option<i64>,
    pub scopes: Option<Vec<String>>,
}

impl fmt::Debug for GrantBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantBody")
            .field("access_token", &"***")
            .field("has_rotated_refresh", &self.rotated_refresh.is_some())
            .field("expires_in", &self.expires_in)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[must_use]
pub fn read_grant_body(body: &TokenResponse) -> Option<GrantBody> {
    let access_token = body
        .get("access_token")
        .and_then(non_empty_text)?
        .to_owned();
    Some(GrantBody {
        access_token,
        rotated_refresh: body
            .get("refresh_token")
            .and_then(non_empty_text)
            .map(str::to_owned),
        expires_in: body.get("expires_in").and_then(parse_expires_in),
        scopes: body.get("scope").and_then(parse_scopes),
    })
}

fn non_empty_text(value: &JsonValue) -> Option<&str> {
    match value {
        JsonValue::String(text) if !text.is_empty() => Some(text),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

// LiteLLM's _parse_expires_in: a boolean is None, an integer is itself, a float and every other type are None.
#[must_use]
pub fn parse_expires_in(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => parse_int_text(text),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

// Python's int() on a string: surrounding whitespace, one optional sign, ASCII digits with single underscores between them.
fn parse_int_text(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, digits) = match (trimmed.strip_prefix('-'), trimmed.strip_prefix('+')) {
        (Some(rest), _) => (true, rest),
        (None, Some(rest)) => (false, rest),
        (None, None) => (false, trimmed),
    };
    if digits.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    let mut after_underscore = true;
    for byte in digits.bytes() {
        match byte {
            b'_' if after_underscore => return None,
            b'_' => after_underscore = true,
            digit @ b'0'..=b'9' => {
                let numeric = i64::from(digit).checked_sub(i64::from(b'0'))?;
                value = value.checked_mul(10)?.checked_add(numeric)?;
                after_underscore = false;
            }
            _ => return None,
        }
    }
    if after_underscore {
        return None;
    }
    if negative {
        return value.checked_neg();
    }
    Some(value)
}

// LiteLLM's _parse_scopes: the words of a non-empty string, None for an empty string or any other type.
#[must_use]
pub fn parse_scopes(value: &JsonValue) -> Option<Vec<String>> {
    match value {
        JsonValue::String(text) if !text.is_empty() => {
            Some(text.split_whitespace().map(str::to_owned).collect())
        }
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}
