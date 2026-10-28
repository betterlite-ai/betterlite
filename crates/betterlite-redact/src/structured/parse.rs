use super::{MAX_WORK, Payload};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("structured payload exceeds the byte limit")]
    TooLarge,
    #[error("structured payload is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_payload(bytes: &[u8]) -> Result<Payload, ParseError> {
    if bytes.len() > MAX_WORK {
        return Err(ParseError::TooLarge);
    }
    let value = serde_json::from_slice(bytes).map_err(|source| ParseError::Json { source })?;
    Ok(value)
}

mod tests;
