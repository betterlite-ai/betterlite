use betterlite_error::error::{ErrorClass, ErrorContext};
use thiserror::Error;

use super::{DROP_PARAMS_HINT, PROVIDER};

#[derive(Debug, Error)]
pub enum SailError {
    #[error(
        "sail does not support service_tier={service_tier}. Supported values: auto, default, priority, flex, balanced. {DROP_PARAMS_HINT}"
    )]
    UnsupportedServiceTier { model: String, service_tier: String },
    #[error(
        "sail does not accept {field}.completion_window. Send service_tier instead. {DROP_PARAMS_HINT}"
    )]
    CallerWindowInMetadata { model: String, field: String },
    #[error(
        "sail does not accept service_tier inside extra_body. Send service_tier instead. {DROP_PARAMS_HINT}"
    )]
    ExtraBodyServiceTier { model: String },
    #[error(
        "sail does not support metadata.completion_window={completion_window}. Supported values: asap, balanced, standard, flex. {DROP_PARAMS_HINT}"
    )]
    UnsupportedCallerWindow {
        model: String,
        completion_window: String,
    },
    #[error(
        "sail got service_tier={service_tier} and metadata.completion_window={completion_window}, which select different completion windows. Send one of them. {DROP_PARAMS_HINT}"
    )]
    ConflictingWindows {
        model: String,
        service_tier: String,
        completion_window: String,
    },
}

impl SailError {
    #[must_use]
    pub fn class(&self) -> ErrorClass {
        ErrorClass::UnsupportedParams(Box::new(ErrorContext {
            message: self.to_string(),
            model: Some(self.model().to_owned()),
            provider: Some(String::from(PROVIDER)),
            max_retries: None,
            num_retries: None,
            body_type: None,
            body_param: None,
            body_code: None,
        }))
    }

    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::UnsupportedServiceTier { .. }
            | Self::CallerWindowInMetadata { .. }
            | Self::ExtraBodyServiceTier { .. }
            | Self::UnsupportedCallerWindow { .. }
            | Self::ConflictingWindows { .. } => 400,
        }
    }

    fn model(&self) -> &str {
        match self {
            Self::UnsupportedServiceTier { model, .. }
            | Self::CallerWindowInMetadata { model, .. }
            | Self::ExtraBodyServiceTier { model }
            | Self::UnsupportedCallerWindow { model, .. }
            | Self::ConflictingWindows { model, .. } => model,
        }
    }
}
