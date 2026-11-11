use super::events::{CONNECTION_TEST_EVENT_NAME, ConnectionTestEvent};
use super::logger::NewRelicLogger;
use crate::exporter_options::health::IntegrationHealthStatus;

const DISABLED_MESSAGE: &str = "New Relic integration is disabled. Check that NEW_RELIC_LICENSE_KEY and NEW_RELIC_APP_NAME are set and the newrelic package is installed.";
const NOT_CONNECTED_MESSAGE: &str = "New Relic Python agent not installed. Review the New Relic integration documentation at https://docs.litellm.ai/docs/observability/newrelic.";

// LiteLLM's async_health_check only records an in-memory event, so this check never awaits
#[must_use]
pub fn async_health_check(logger: &NewRelicLogger) -> IntegrationHealthStatus {
    if !logger.enabled {
        return IntegrationHealthStatus::unhealthy(DISABLED_MESSAGE.to_owned());
    }
    let resolved = logger.agent.as_ref().and_then(|agent| agent.application());
    let Some(application) = resolved.filter(|candidate| candidate.enabled()) else {
        return IntegrationHealthStatus::unhealthy(NOT_CONNECTED_MESSAGE.to_owned());
    };
    let event = ConnectionTestEvent {
        app_name: logger.settings.app_name.clone(),
        timestamp: (logger.clock)().as_secs_f64(),
    };
    match application.record_custom_event(CONNECTION_TEST_EVENT_NAME, &event.attributes()) {
        Ok(()) => IntegrationHealthStatus::healthy(),
        Err(error) => IntegrationHealthStatus::unhealthy(error.to_string()),
    }
}
