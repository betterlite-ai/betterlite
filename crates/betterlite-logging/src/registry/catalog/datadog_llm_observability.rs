use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::datadog::apm::NullTracer;
use crate::datadog_llm_metrics::token_counting::ToolTokenCounter;
use crate::datadog_llm_observability::logger::{DatadogLlmObsDeps, DatadogLlmObsLogger};
use crate::datadog_llm_observability::settings::{DatadogLlmObsInit, DatadogLlmObsInitParams};
use crate::datadog_logging::logger::uuid_id;
use crate::datadog_logging::transport::PooledTransport;

mod tests;

// B196's tokenizer depends transitively on this crate, so the catalog has no counter to plug in yet
pub struct UnwiredCounter;

impl ToolTokenCounter for UnwiredCounter {
    type Failure = Infallible;

    fn count_text(
        &self,
        _model: &str,
        _text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(Ok(0))
    }
}

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let logger = concrete_logger(inputs)?;
    let callback: Arc<dyn CallbackLogger> = logger;
    Ok(callback)
}

fn concrete_logger(
    inputs: &CatalogInputs,
) -> Result<Arc<DatadogLlmObsLogger<UnwiredCounter>>, CatalogError> {
    let global_turn_off = inputs
        .config
        .sections
        .logging_privacy
        .turn_off_message_logging;
    let init = DatadogLlmObsInit {
        dd_api_key: None,
        dd_site: None,
        dd_agent_host: None,
        dd_agent_port: None,
        turn_off_message_logging: None,
        message_logging: None,
        batch_size: None,
        flush_interval_seconds: None,
        params: init_params(inputs)?,
        global_turn_off,
    };
    let logger = DatadogLlmObsLogger::new(
        &init,
        inputs.env.as_ref(),
        Arc::new(UnwiredCounter),
        DatadogLlmObsDeps {
            transport: Arc::new(PooledTransport {
                client: inputs.http.clone(),
            }),
            tracer: Arc::new(NullTracer),
            ids: uuid_id,
            tasks: Some(inputs.tracker.clone()),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(logger)
}

// litellm_settings.datadog_llm_observability_params rides the per-callback settings map here
fn init_params(inputs: &CatalogInputs) -> Result<Option<DatadogLlmObsInitParams>, CatalogError> {
    let name = CallbackName::DatadogLlmObservability.as_str();
    let Some(entry): Option<&CallbackEntry> = inputs.callback_settings.get(name) else {
        return Ok(None);
    };
    let mut params = DatadogLlmObsInitParams::default();
    for (field, value) in entry.exposed_variables() {
        match field.as_str() {
            "turn_off_message_logging" => {
                params.turn_off_message_logging =
                    betterlite_env::flag(&field, &value).map_err(|error| CatalogError::Init {
                        message: error.to_string(),
                    })?;
            }
            // pydantic's DatadogLLMObsInitParams drops a key it does not declare
            dropped => tracing::debug!(field = %dropped, "the dropped init setting is ignored"),
        }
    }
    Ok(Some(params))
}
