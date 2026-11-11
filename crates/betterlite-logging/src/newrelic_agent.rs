pub mod agent;
pub mod backoff;
pub mod events;
pub mod health;
pub mod init;
pub mod logger;
pub mod metrics;
pub mod success;

mod tests;

use crate::newrelic_extraction::ExtractionError;

#[derive(Debug, thiserror::Error)]
pub enum NewRelicAgentError {
    #[error("the New Relic agent could not register the application: {message}")]
    Registration { message: String },
    #[error("the New Relic agent could not record a custom event: {message}")]
    Event { message: String },
    #[error("the New Relic agent could not record a custom metric: {message}")]
    Metric { message: String },
    #[error("the New Relic event inputs could not be read")]
    Extraction(#[source] ExtractionError),
}
