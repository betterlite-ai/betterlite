use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_models::decisions::Json;

use super::super::decode_discovery_document;

// get_public_key's split: every comma-separated URL trimmed, and a blank entry dropped rather than fetched.
#[must_use]
pub fn plain_urls(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .collect()
}

// parse_keys quirks: one dict or one list accepts the kid, a longer list takes the last match, a longer dict none.
pub fn parse_keys(keys: &Json, kid: Option<&str>) -> Option<Json> {
    match keys {
        Json::List(items) => match items.as_slice() {
            [only] => single_key_match(only, kid).then(|| only.clone()),
            _ if items.len() > 1 => {
                let mut found = None;
                for item in items {
                    let Json::Object(fields) = item else {
                        continue;
                    };
                    let matched = kid.is_some_and(|wanted| {
                        matches!(fields.get("kid"), Some(Json::Text(value)) if value == wanted)
                    });
                    if matched {
                        found = Some(item.clone());
                    }
                }
                found
            }
            _ => None,
        },
        Json::Object(fields) if fields.len() == 1 => (kid.is_none()
            || matches!(fields.get("kid"), Some(Json::Text(value)) if Some(value.as_str()) == kid))
        .then(|| keys.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => None,
        Json::Object(_) => None,
    }
}

fn single_key_match(only: &Json, kid: Option<&str>) -> bool {
    // a one-entry list of something that is not a key object is no key; LiteLLM raises on its .get instead
    let Json::Object(fields) = only else {
        return false;
    };
    kid.is_none()
        || matches!(fields.get("kid"), Some(Json::Text(value)) if Some(value.as_str()) == kid)
}

// is_allowed_domain: unset allows every address, else the text after the last @ must match exactly.
#[must_use]
pub fn is_allowed_domain(allowed: Option<&str>, user_email: &str) -> bool {
    let Some(allowed) = allowed else {
        return true;
    };
    user_email
        .rsplit('@')
        .next()
        .is_some_and(|domain| domain == allowed)
}

// The kid of a compact JWT header, which pyjwt only hands over when the header is a JSON object with a string kid.
#[must_use]
pub fn token_kid(token: &str) -> Option<String> {
    let mut segments = token.split('.');
    let header = segments.next()?;
    segments.next()?;
    segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let padding = header
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    if padding > 2 {
        return None;
    }
    let unpadded = header.get(..header.len().checked_sub(padding)?)?;
    let bytes = URL_SAFE_NO_PAD.decode(unpadded).ok()?;
    let Json::Object(fields) = decode_discovery_document(&bytes).ok()? else {
        return None;
    };
    match fields.get("kid") {
        Some(Json::Text(kid)) => Some(kid.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
