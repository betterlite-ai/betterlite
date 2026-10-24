use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BedrockError {
    #[error("requestMetadata must be an object")]
    MetadataNotObject,
    #[error("requestMetadata has {count} pairs, Bedrock allows at most 16")]
    TooManyMetadataPairs { count: usize },
    #[error("requestMetadata value for key {key:?} must be a string")]
    MetadataValueNotText { key: String },
    #[error("requestMetadata keys must be 1 to 256 characters long, got {length}")]
    MetadataKeyLength { length: usize },
    #[error("requestMetadata value for key {key:?} is longer than 256 characters")]
    MetadataValueLength { key: String },
    #[error("requestMetadata key {key:?} has characters Bedrock does not allow")]
    MetadataKeyCharacters { key: String },
    #[error("requestMetadata value for key {key:?} has characters Bedrock does not allow")]
    MetadataValueCharacters { key: String },
}

impl BedrockError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MetadataNotObject
            | Self::TooManyMetadataPairs { .. }
            | Self::MetadataValueNotText { .. }
            | Self::MetadataKeyLength { .. }
            | Self::MetadataValueLength { .. }
            | Self::MetadataKeyCharacters { .. }
            | Self::MetadataValueCharacters { .. } => 400,
        }
    }
}
