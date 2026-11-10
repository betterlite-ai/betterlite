use std::error::Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChargeError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error("the budget duration {spec} cannot be read as a number of seconds")]
    Duration { spec: String },
    #[error("the spend counter for {spend_key} was not incremented")]
    Increment {
        spend_key: String,
        #[source]
        source: E,
    },
}
