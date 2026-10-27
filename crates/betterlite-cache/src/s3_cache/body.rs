use super::entry::{CachedEntry, literal};

pub fn decode_body(bytes: &[u8]) -> Result<CachedEntry, DecodeError> {
    let value = match serde_json::from_slice(bytes) {
        Ok(value) => CachedEntry(value),
        Err(_) => CachedEntry(
            literal::parse(bytes).map_err(|source| DecodeError::Literal {
                source: Box::new(source),
            })?,
        ),
    };
    if value.0.is_object() {
        return Ok(value);
    }
    literal::to_dictionary(&value.0)
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("invalid cache JSON")]
    Json {
        #[source]
        source: Box<serde_json::Error>,
    },
    #[error("invalid Python cache literal")]
    Literal {
        #[source]
        source: Box<literal::LiteralError>,
    },
    #[error("cache value cannot convert to a JSON dictionary")]
    Dictionary,
}
