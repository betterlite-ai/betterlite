mod tests;

use std::sync::Arc;

use betterlite_config::sections::callback_settings::names::CallbackName;
use betterlite_context::service_trace_id_cache::ServiceTraceIDCache;
use tokio::runtime::Handle;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::langfuse_logger::budget::LangfuseClientBudget;
use crate::langfuse_logger::{AssemblyDefaults, HttpClientBuilder, LangfuseServices};
use crate::langfuse_observations::requested_ids::RequestedIdGenerator;
use crate::langfuse_prompt_management::bridge::run_async_function;
use crate::langfuse_prompt_management::{LangfusePromptManagement, unix_seconds};
use crate::langfuse_rest::cache::SystemClock;

// the single Langfuse logger: prompt management plus the success and failure callbacks
pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if !configured(inputs) {
        return Err(CatalogError::Unavailable {
            name: CallbackName::Langfuse.as_str().to_owned(),
        });
    }
    let services = services(inputs)?;
    let trace_cache = Arc::new(ServiceTraceIDCache::new(Arc::new(unix_seconds)).map_err(
        |error| CatalogError::Init {
            message: error.to_string(),
        },
    )?);
    let manager = run_async_function(LangfusePromptManagement::new(services, trace_cache))
        .map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?
        .map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?;
    Ok(Arc::new(manager))
}

// set_callbacks only builds the callbacks the config lists, in any of the three lists
fn configured(inputs: &CatalogInputs) -> bool {
    let name = CallbackName::Langfuse.as_str();
    if inputs.callback_settings.contains_key(name) {
        return true;
    }
    let settings = &inputs.config.sections.callback_settings;
    [
        &settings.callbacks,
        &settings.success_callback,
        &settings.failure_callback,
    ]
    .iter()
    .any(|names| names.iter().any(|listed| listed == name))
}

fn services(inputs: &CatalogInputs) -> Result<LangfuseServices, CatalogError> {
    let runtime = Handle::try_current().map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let budget = LangfuseClientBudget::from_environment(inputs.env.as_ref()).map_err(|error| {
        CatalogError::Init {
            message: error.to_string(),
        }
    })?;
    Ok(LangfuseServices {
        settings: inputs.config.sections.langfuse_settings.clone(),
        environment: Arc::clone(&inputs.env),
        http: inputs.http.clone(),
        budget,
        tasks: inputs.tracker.clone(),
        runtime,
        assembly: AssemblyDefaults {
            redact_user_api_key_info: inputs
                .config
                .sections
                .logging_privacy
                .redact_user_api_key_info,
            ..AssemblyDefaults::default()
        },
        id_generator: Arc::new(RequestedIdGenerator::default()),
        clients: Arc::new(HttpClientBuilder {
            http: inputs.http.clone(),
            clock: Arc::new(SystemClock),
            tasks: inputs.tracker.clone(),
        }),
    })
}
