use std::sync::Arc;

use thiserror::Error;

// the five values router_cooldown_event_callback hands the Prometheus logger
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CooldownEvent<'event> {
    pub model_name: &'event str,
    pub model_id: &'event str,
    pub api_base: &'event str,
    pub provider: &'event str,
    pub exception_status: &'event str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CooldownObservationError {
    #[error("{message}")]
    Observer { message: String },
}

// CustomLogger's cooldown hook: the base class exports nothing, and an observer overrides what it handles
pub trait CooldownObserver: Send + Sync {
    fn deployment_cooled_down(
        &self,
        _event: &CooldownEvent<'_>,
    ) -> Result<(), CooldownObservationError> {
        Ok(())
    }
}

// router_cooldown_event_callback: every observer sees the cooldown, and one observer's failure is logged and skipped
pub fn notify(observers: &[Arc<dyn CooldownObserver>], event: &CooldownEvent<'_>) {
    for observer in observers {
        if let Err(error) = observer.deployment_cooled_down(event) {
            tracing::error!("Error in router_cooldown_event_callback: {error}");
        }
    }
}
