use std::collections::BTreeMap;

use betterlite_anthropic::anthropic_capability_catalog::sampling::SamplingError;
use betterlite_anthropic::capabilities::speed::SpeedError;
use betterlite_error::error::details::{RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;

#[derive(Debug, thiserror::Error)]
pub enum MessagesOptionsError {
    #[error(transparent)]
    Speed(#[from] SpeedError),
    #[error(transparent)]
    Sampling(#[from] SamplingError),
    #[error("this is a mock internal server error")]
    MockInternalServer { model: String },
    #[error("this is a mock context window exceeded error")]
    MockContextWindowExceeded { model: String },
    #[error("this is a mock rate limit error")]
    MockRateLimit { model: String },
}

impl MessagesOptionsError {
    // The SDK exception LiteLLM raises; None when the gate could not render the rejected value.
    #[must_use]
    pub fn classify(&self) -> Option<ErrorClass> {
        match self {
            Self::Speed(error) => error.classify(),
            Self::Sampling(error) => error.classify(),
            Self::MockInternalServer { model } => Some(ErrorClass::InternalServer(Box::new(
                self.mock_context(model),
            ))),
            Self::MockContextWindowExceeded { model } => Some(ErrorClass::ContextWindowExceeded(
                Box::new(ResponseDetails::from(self.mock_context(model))),
            )),
            Self::MockRateLimit { model } => {
                Some(ErrorClass::RateLimit(Box::new(RateLimitDetails {
                    context: self.mock_context(model),
                    category: RateLimitCategory::VendorRateLimit,
                    dimension: None,
                    headers: BTreeMap::new(),
                })))
            }
        }
    }

    fn mock_context(&self, model: &str) -> ErrorContext {
        ErrorContext {
            message: self.to_string(),
            model: Some(model.to_owned()),
            provider: Some("anthropic".to_owned()),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }
    }
}
