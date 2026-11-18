use crate::otel::v2_callback::error::CallbackError;

#[derive(Debug, thiserror::Error)]
pub enum LoggerRuntimeError {
    #[error("the OTel V2 logger constructor failed: {source}")]
    Constructor {
        #[source]
        source: CallbackError,
    },
}
