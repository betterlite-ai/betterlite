use serde::de::DeserializeOwned;
use serde_json::Deserializer;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WireError {
    #[error(transparent)]
    Shape(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("the body has bytes after the JSON document: {0}")]
    Trailing(#[from] serde_json::Error),
}

impl WireError {
    // A data error means the body was valid JSON of the wrong shape; a syntax error means it was not JSON at all.
    #[must_use]
    pub fn is_data_error(&self) -> bool {
        match self {
            Self::Shape(source) => source.inner().is_data(),
            Self::Trailing(_source) => false,
        }
    }
}

pub fn decode<T>(bytes: &[u8]) -> Result<T, WireError>
where
    T: DeserializeOwned,
{
    let mut decoder = Deserializer::from_slice(bytes);
    let value: T = serde_path_to_error::deserialize(&mut decoder)?;
    decoder.end()?;
    Ok(value)
}
