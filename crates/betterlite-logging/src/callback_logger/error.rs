use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoggerError {
    #[error("callback {callback} failed: {message}")]
    Delivery { callback: String, message: String },
}
