use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_config::types::Config;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_models::decisions::Json;
use betterlite_router::prompt_affinity::service::{PromptAffinity, SuccessEvent};
use betterlite_router::router_core::services::RouteServices;

use crate::routing::prompt_affinity::{PromptAffinityError, PromptAffinityInputs, build};

// the callback name LiteLLM reports for the check, its class name
const CALLBACK_NAME: &str = "PromptCachingDeploymentCheck";

// The dynamic state a server route reads, and the callback its successes are logged to.
pub struct RouterServices {
    prompt_affinity: Option<PromptAffinity>,
}

impl RouterServices {
    // Router.add_optional_pre_call_checks: the prompt caching check exists only when the config names it
    pub fn from_config<Memory: MemoryCacheOps + 'static>(
        config: &Config,
        inputs: PromptAffinityInputs<Memory>,
    ) -> Result<Self, PromptAffinityError> {
        Ok(Self {
            prompt_affinity: build(&config.sections.prompt_affinity, inputs)?,
        })
    }
}

impl RouteServices for RouterServices {
    fn prompt_affinity(&self) -> Option<&PromptAffinity> {
        self.prompt_affinity.as_ref()
    }
}

// PromptCachingDeploymentCheck.async_log_success_event: the deployment that served this prompt is pinned
#[async_trait]
impl CallbackLogger for RouterServices {
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let Some(affinity) = &self.prompt_affinity else {
            return Ok(());
        };
        let payload = &event.payload;
        let Some(Json::List(messages)) = &payload.messages else {
            return Ok(());
        };
        let Some(model_id) = payload.model_id.as_deref() else {
            return Ok(());
        };
        affinity
            .record_success(&SuccessEvent {
                call_type: payload.call_type.as_str(),
                model: payload.model.as_str(),
                messages: Some(messages.as_slice()),
                model_id: Some(model_id),
            })
            .await
            .map_err(|error| LoggerError::Delivery {
                callback: CALLBACK_NAME.to_owned(),
                message: error.to_string(),
            })
    }
}
