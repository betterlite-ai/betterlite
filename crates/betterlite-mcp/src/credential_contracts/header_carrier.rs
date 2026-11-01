mod tests;

use super::errors::{CredError, CredResult};

pub const DEFAULT_CREDENTIAL_HEADER: &str = "Authorization";
const DEFAULT_VALUE_PREFIX: &str = "Bearer";

// RFC 7230 token characters outside [0-9A-Za-z]: !#$%&'*+-.^_`|~
const HEADER_NAME_EXTRA_TOKEN_BYTES: &[u8] = b"!#$%&'*+-.^_`|~";

pub fn validate_header_name(raw: &str) -> CredResult<String> {
    let trimmed = raw.trim();
    let is_token = trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || HEADER_NAME_EXTRA_TOKEN_BYTES.contains(&byte));
    if trimmed.is_empty() || !is_token {
        return Err(CredError::Misconfigured(format!(
            "invalid upstream header name: '{raw}'"
        )));
    }
    Ok(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderCarrier {
    pub header_name: String,
    pub value_prefix: String,
}

impl Default for HeaderCarrier {
    fn default() -> Self {
        Self {
            header_name: DEFAULT_CREDENTIAL_HEADER.to_owned(),
            value_prefix: DEFAULT_VALUE_PREFIX.to_owned(),
        }
    }
}

impl HeaderCarrier {
    #[must_use]
    pub fn header(&self, value: &str) -> (String, String) {
        let formatted = if self.value_prefix.is_empty() {
            value.to_owned()
        } else {
            format!("{} {value}", self.value_prefix)
        };
        (self.header_name.clone(), formatted)
    }
}
