use serde::de::DeserializeOwned;
use serde::de::IgnoredAny;
use serde_json::Deserializer as JsonDeserializer;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WireError {
    #[error(transparent)]
    Shape(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("the body has bytes after the JSON document: {0}")]
    Trailing(#[from] serde_json::Error),
    #[error("the JSON body is an array or scalar, not an object")]
    NotObject,
}

impl WireError {
    // A data error means the body was valid JSON of the wrong shape; a syntax error means it was not JSON at all.
    #[must_use]
    pub fn is_data_error(&self) -> bool {
        match self {
            Self::Shape(source) => source.inner().is_data(),
            Self::Trailing(_source) => false,
            Self::NotObject => true,
        }
    }
}

pub fn decode<T>(bytes: &[u8]) -> Result<T, WireError>
where
    T: DeserializeOwned,
{
    let mut decoder = JsonDeserializer::from_slice(bytes);
    let value: T = serde_path_to_error::deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}

// serde_json fills a struct's fields from the elements of a JSON array, so ["detail"] would decode as a detail body; LiteLLM reads these bodies as objects only.
pub fn decode_object<T>(bytes: &[u8]) -> Result<T, WireError>
where
    T: DeserializeOwned,
{
    if is_array_or_scalar(bytes) {
        return Err(WireError::NotObject);
    }
    decode(bytes)
}

fn is_array_or_scalar(bytes: &[u8]) -> bool {
    // JSON allows only space, tab, carriage return and line feed as leading whitespace.
    let opens_object = bytes
        .iter()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        .is_none_or(|byte| *byte == b'{');
    !opens_object && serde_json::from_slice::<IgnoredAny>(bytes).is_ok()
}
