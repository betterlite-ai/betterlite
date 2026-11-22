pub mod collectors;
pub mod counters;
pub mod error;
pub mod failure;
pub mod fallbacks;
pub mod metric;
pub mod metrics;
pub mod requests;
pub mod state;
pub mod success;
mod values;

#[cfg(test)]
#[path = "prometheus_deployment/tests/callback_counters.rs"]
mod callback_counters;
#[cfg(test)]
#[path = "prometheus_deployment/tests/deployment_state.rs"]
mod deployment_state;
#[cfg(test)]
#[path = "prometheus_deployment/tests/fallback_events.rs"]
mod fallback_events;
#[cfg(test)]
#[path = "prometheus_deployment/tests/failure_outcome.rs"]
mod failure_outcome;
#[cfg(test)]
#[path = "prometheus_deployment/tests/success_outcome.rs"]
mod success_outcome;
#[cfg(test)]
#[path = "prometheus_deployment/tests/support.rs"]
mod support;
