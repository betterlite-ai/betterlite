use std::sync::Arc;

use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_models::model_registration::JsonObject;
use thiserror::Error;

// the three values log_success_fallback_event and log_failure_fallback_event hand every CustomLogger
#[derive(Debug, Clone, Copy)]
pub struct FallbackEvent<'event> {
    pub original_model_group: &'event str,
    pub request: &'event JsonObject,
    pub exception: &'event ErrorClass,
}

// which of the two CustomLogger hooks a dispatch calls
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackOutcome {
    Success,
    Failure,
}

impl FallbackOutcome {
    #[must_use]
    pub const fn handler_name(self) -> &'static str {
        match self {
            Self::Success => "log_success_fallback_event",
            Self::Failure => "log_failure_fallback_event",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FallbackObservationError {
    #[error("{message}")]
    Observer { message: String },
}

// CustomLogger's two fallback hooks: the base class does nothing for either, and an observer overrides what it handles
#[async_trait]
pub trait FallbackObserver: Send + Sync {
    async fn log_success_fallback_event(
        &self,
        _event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Ok(())
    }

    async fn log_failure_fallback_event(
        &self,
        _event: FallbackEvent<'_>,
    ) -> Result<(), FallbackObservationError> {
        Ok(())
    }
}

// log_success_fallback_event/log_failure_fallback_event: every logger sees the event, and one logger's failure is logged and skipped
pub async fn log_fallback_event(
    observers: &[Arc<dyn FallbackObserver>],
    outcome: FallbackOutcome,
    event: FallbackEvent<'_>,
) {
    for observer in observers {
        let result = match outcome {
            FallbackOutcome::Success => observer.log_success_fallback_event(event).await,
            FallbackOutcome::Failure => observer.log_failure_fallback_event(event).await,
        };
        if let Err(error) = result {
            tracing::error!("Error in {}: {error}", outcome.handler_name());
        }
    }
}
