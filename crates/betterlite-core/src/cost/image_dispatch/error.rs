use std::convert::Infallible;
use std::error::Error;

use betterlite_models::token_rates::TokenRateError;
use thiserror::Error;

// E is the failure type of the installed price policy; steps before the policy runs cannot raise it.
#[derive(Debug, Error)]
pub enum ImageDispatchError<E = Infallible>
where
    E: Error + 'static,
{
    #[error("the image size pattern does not compile")]
    Pattern {
        #[source]
        source: regex::Error,
    },
    #[error("deployment prices are not readable")]
    Rates {
        #[source]
        source: TokenRateError,
    },
    #[error("deployment prices cannot be written as JSON numbers")]
    Prices {
        #[source]
        source: serde_json::Error,
    },
    #[error("completion_response must be of type ImageResponse for bedrock image cost calculation")]
    NotImageResponse,
    #[error(transparent)]
    Policy(E),
}
