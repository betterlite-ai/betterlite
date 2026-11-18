use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_config::types::Config;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_models::decisions::Json;
use betterlite_router::prompt_affinity::service::{PromptAffinity, SuccessEvent};
use betterlite_router::router_core::attempt::AttemptResult;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_core::pipeline::{self, Resolved};
use betterlite_router::router_core::route::{
    ClientAccess, RouteContext, RoutePolicy, RouteRequest,
};
use betterlite_router::router_core::services::RouteServices;
use betterlite_router::router_predicates::params::ParamSupport;
use betterlite_router::static_eligibility::tokens::TokenLimits;
use betterlite_router::strategy_dispatch::service::StrategyDispatch;
use rand::Rng;

use crate::routing::prompt_affinity::{PromptAffinityError, PromptAffinityInputs, build};
use crate::routing::routing_state::RoutingState;

mod tests;

// the callback name LiteLLM reports for the check, its class name
const CALLBACK_NAME: &str = "PromptCachingDeploymentCheck";

// The dynamic state a server route reads, and the callback its successes are logged to.
pub struct RouterServices {
    prompt_affinity: Option<PromptAffinity>,
}

impl RouterServices {
    // Router.add_optional_pre_call_checks: the prompt caching check exists only when the config names it
    pub fn from_config<Memory>(
        config: &Config,
        inputs: PromptAffinityInputs<Memory>,
    ) -> Result<Self, PromptAffinityError>
    where
        Memory: MemoryCacheOps + 'static,
    {
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

// Everything one server route reads besides the catalog's answer: the request, the typed services and the settings.
#[derive(Clone, Copy)]
pub struct RouteInputs<'route> {
    pub request: &'route RouteRequest<'route>,
    pub limits: &'route (dyn TokenLimits + Sync),
    pub params: &'route (dyn ParamSupport + Sync),
    pub policy: RoutePolicy<'route>,
    pub clients: Option<ClientAccess<'route>>,
}

// Router.async_get_available_deployment over one server route: every stage reads the config's rules through the routing state.
pub async fn route<R>(
    state: &RoutingState,
    inputs: RouteInputs<'_>,
    dispatch: &mut StrategyDispatch,
    resolved: Resolved,
    random: &mut R,
) -> Result<AttemptResult, RoutingError>
where
    R: Rng + Send,
{
    let context = RouteContext {
        request: inputs.request,
        services: state,
        limits: inputs.limits,
        params: inputs.params,
        policy: inputs.policy,
        clients: inputs.clients,
    };
    pipeline::route(&context, dispatch, resolved, random).await
}
