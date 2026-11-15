use thiserror::Error;

#[derive(Debug, Error)]
pub enum XaiRealtimeError {
    #[error("the Grok Voice session cached more than {limit} content parts")]
    ContentPartLimit { limit: usize },
}
