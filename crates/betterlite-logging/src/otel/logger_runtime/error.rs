#[derive(Debug, thiserror::Error)]
pub enum LoggerRuntimeError {
    #[error("the OTel V2 logger constructor failed: {message}")]
    Constructor { message: String },
}
