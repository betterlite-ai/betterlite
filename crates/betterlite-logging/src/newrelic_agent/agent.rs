use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::NewRelicAgentError;

// The Python module boundary: newrelic.agent.register_application and newrelic.agent.application
pub trait NewRelicAgent: Send + Sync {
    fn register_application(&self, timeout: Duration) -> Result<(), NewRelicAgentError>;

    fn application(&self) -> Option<Arc<dyn NewRelicApplication>>;
}

pub trait NewRelicApplication: Send + Sync {
    fn enabled(&self) -> bool;

    fn record_custom_event(
        &self,
        name: &str,
        attributes: &IndexMap<String, Json>,
    ) -> Result<(), NewRelicAgentError>;

    fn record_custom_metric(&self, name: &str, value: i64) -> Result<(), NewRelicAgentError>;
}
