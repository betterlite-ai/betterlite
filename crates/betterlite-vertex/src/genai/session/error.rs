use crate::google_native_content::error::NativeContentError;
use betterlite_config::sections::vertex_credential_sources::credential_source::CredentialDecodeError;

// What the provider config reports while B3489 builds the body, the URL and the auth headers
#[derive(Debug, thiserror::Error)]
pub enum GenaiConfigError {
    #[error("the generate content request could not be built: {source}")]
    Native {
        #[source]
        source: NativeContentError,
    },
    #[error("the generate content body could not be encoded: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the vertex credentials could not be read: {source}")]
    Credentials {
        #[source]
        source: CredentialDecodeError,
    },
}

impl From<NativeContentError> for GenaiConfigError {
    fn from(source: NativeContentError) -> Self {
        Self::Native { source }
    }
}
