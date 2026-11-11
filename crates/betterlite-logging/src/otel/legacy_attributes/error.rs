use thiserror::Error;

#[derive(Debug, Error)]
pub enum LegacyAttributesError {
    #[error("standard_logging_object not found in kwargs")]
    MissingStandardLoggingObject,
}

mod tests;
