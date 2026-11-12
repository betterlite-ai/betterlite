mod adapter;
mod payload;
mod tests;

use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::deepeval::deepeval_logging::environment::EnvironmentError;
use crate::deepeval::deepeval_logging::ids::SystemUuids;
use crate::deepeval::deepeval_logging::{DeepEvalError, DeepEvalLogger};
use crate::http_analytics_loggers::delivery::PooledDelivery;
use adapter::DeepevalAdapter;

// the merged settings take any sized lookup; the catalog hands a trait object
struct Lookup<'env>(&'env (dyn EnvironmentLookup + Send + Sync));

impl EnvironmentLookup for Lookup<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.lookup(name)
    }
}

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let logger = DeepEvalLogger::from_environment(
        &Lookup(inputs.env.as_ref()),
        PooledDelivery {
            client: inputs.http.clone(),
        },
        SystemUuids,
    )
    .map_err(|error| CatalogError::Init {
        message: init_message(&error),
    })?;
    Ok(Arc::new(DeepevalAdapter {
        logger: Arc::new(logger),
        tracker: inputs.tracker.clone(),
    }))
}

// LiteLLM raises this text while building the logger, which set_callbacks reports and then skips it
fn init_message<E>(error: &DeepEvalError<E>) -> String
where
    E: StdError + 'static,
{
    match error {
        DeepEvalError::MissingApiKey => {
            "Please set 'CONFIDENT_API_KEY=<>' in your environment variables.".to_owned()
        }
        DeepEvalError::Environment(EnvironmentError::Invalid { environment }) => format!(
            "Invalid environment: {environment}. Please use one of the following instead: \"production\", \"development\", \"staging\""
        ),
        DeepEvalError::Api(_) => error.to_string(),
    }
}
