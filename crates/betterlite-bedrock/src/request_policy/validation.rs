use crate::error::BedrockError;
use crate::request_policy::RequestValue;

pub(crate) const MAX_PAIRS: usize = 16;
const MAX_LENGTH: usize = 256;

pub fn validate_request_metadata(metadata: &RequestValue) -> Result<(), BedrockError> {
    let RequestValue::Mapping(pairs) = metadata else {
        return Err(BedrockError::MetadataNotObject);
    };
    if pairs.len() > MAX_PAIRS {
        return Err(BedrockError::TooManyMetadataPairs { count: pairs.len() });
    }
    for (key, value) in pairs {
        let RequestValue::Text(text) = value else {
            return Err(BedrockError::MetadataValueNotText { key: key.clone() });
        };
        let length = key.chars().count();
        if length == 0 || length > MAX_LENGTH {
            return Err(BedrockError::MetadataKeyLength { length });
        }
        if text.chars().count() > MAX_LENGTH {
            return Err(BedrockError::MetadataValueLength { key: key.clone() });
        }
        if !key.chars().all(is_allowed) {
            return Err(BedrockError::MetadataKeyCharacters { key: key.clone() });
        }
        if !text.chars().all(is_allowed) {
            return Err(BedrockError::MetadataValueCharacters { key: key.clone() });
        }
    }
    Ok(())
}

pub(crate) fn is_forwardable(key: &str, value: &str) -> bool {
    let key_length = key.chars().count();
    (1..=MAX_LENGTH).contains(&key_length)
        && value.chars().count() <= MAX_LENGTH
        && key.chars().chain(value.chars()).all(is_allowed)
}

const fn is_allowed(character: char) -> bool {
    match character {
        // Python's \s also matches the separators U+001C to U+001F, Rust's is_whitespace does not
        '\u{1c}'..='\u{1f}' | ':' | '_' | '@' | '$' | '#' | '=' | '/' | '+' | ',' | '.' | '-' => {
            true
        }
        other => other.is_ascii_alphanumeric() || other.is_whitespace(),
    }
}
