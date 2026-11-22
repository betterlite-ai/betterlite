use betterlite_openai::compatible_wire::error::CompatibleWireError;
use betterlite_openai::error::OpenAiError;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;

use crate::converse_request::error::ConverseRequestError;

#[derive(Debug, thiserror::Error)]
pub enum NovaChatError {
    #[error(transparent)]
    Converse(#[from] ConverseRequestError),
    #[error(transparent)]
    Reply(#[from] CompatibleWireError),
    #[error(transparent)]
    Config(#[from] OpenAiError),
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
    #[error("the invoke body field {field} must be {expected}")]
    BodyField {
        field: &'static str,
        expected: &'static str,
    },
}
