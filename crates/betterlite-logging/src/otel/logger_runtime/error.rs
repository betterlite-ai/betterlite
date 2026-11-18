use betterlite_error::normalize::NormalizeError;

use crate::otel::v2_callback::error::CallbackError;

#[derive(Debug, thiserror::Error)]
pub enum LoggerRuntimeError {
    #[error("the OTel V2 logger constructor failed: {0}")]
    Constructor(#[from] CallbackError),
    #[error("the OTel error normalizer could not be built")]
    Normalizer(#[from] NormalizeError),
    #[error("the Langfuse content logger needs the runtime's stream output assembler")]
    MissingAssembler,
}
