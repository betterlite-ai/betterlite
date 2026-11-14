use std::convert::Infallible;
use std::error::Error;

use thiserror::Error;

// CallbackError is the failure type of a callback's predicate; steps before a predicate runs cannot raise it.
#[derive(Debug, Error)]
pub enum RealtimeGuardrailPolicyError<CallbackError = Infallible>
where
    CallbackError: Error + 'static,
{
    #[error("the realtime setup's {field} is not a JSON object")]
    SetupFieldNotObject { field: &'static str },
    #[error("the realtime session update cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Callback(#[from] CallbackError),
}
