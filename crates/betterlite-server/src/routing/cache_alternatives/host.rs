use async_trait::async_trait;
use betterlite_anthropic::cache_aware_routing::AnthropicCacheRouting;
use betterlite_anthropic::cache_aware_routing::services::PredictionServices;
use betterlite_anthropic::cache_prediction_targets::counting::TokenCounter;
use betterlite_anthropic::count_tokens::handler::{
    AnthropicCountTokensHandler, CountTokensTransport,
};
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_prompts::message_content::Message;
use betterlite_rate_limit::pre_call_hook::{
    PreCallHookError, RateLimitPreCallHook, RequestCapacityGuard,
};
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_core::error::RoutingError;
use betterlite_router::router_predicates::JsonMap;
use secrecy::SecretString;

use crate::routing::cache_alternatives::config::CacheRoutingConfig;
use crate::routing::cache_alternatives::error::CacheAlternativesError;

// Everything cache-aware selection reads from the live proxy, one method per source the caller owns.
#[async_trait]
pub trait CacheRoutingHost: Send + Sync {
    type Transport: CountTokensTransport;

    fn routing_config(&self) -> CacheRoutingConfig<'_>;
    fn prediction_services(&self) -> PredictionServices<'_>;
    fn price_lookup(&self) -> &dyn PriceLookup;
    fn count_tokens_handler(&self) -> &AnthropicCountTokensHandler<Self::Transport>;
    fn capacity_limiter(&self) -> Option<&dyn CapacityLimiter>;
    fn clock_seconds(&self) -> f64;
    fn proxy_uses_this_router(&self) -> bool;
    fn has_routing_plugins(&self) -> bool;
    fn has_request_transforms(&self) -> bool;
    fn compaction_pending(&self, request_kwargs: &JsonMap) -> bool;
    fn deployments_for_request(
        &self,
        model: &str,
        request_kwargs: &JsonMap,
    ) -> Result<Vec<DeploymentSpec>, CatalogLookupError>;
    async fn healthy_deployments(
        &self,
        model: &str,
        messages: Option<&[Message]>,
        request_kwargs: &JsonMap,
    ) -> Result<Vec<DeploymentSpec>, RoutingError>;
    async fn can_key_call_model(
        &self,
        caller: &UserAPIKeyAuth,
        model: &str,
    ) -> Result<(), EntityModelAccessError>;
}

// The capacity a count holds while it runs; a guard is released when the slot drops.
pub trait CapacitySlot: Send {}

impl CapacitySlot for RequestCapacityGuard {}

// _PROXY_MaxParallelRequestsHandler_v3.request_capacity, so a fake limiter can stand in for the proxy hook.
#[async_trait]
pub trait CapacityLimiter: Send + Sync {
    async fn capacity(
        &self,
        caller: &UserAPIKeyAuth,
        model: &str,
        request_data: &JsonMap,
    ) -> Result<Box<dyn CapacitySlot>, PreCallHookError>;
}

#[async_trait]
impl CapacityLimiter for RateLimitPreCallHook {
    async fn capacity(
        &self,
        caller: &UserAPIKeyAuth,
        model: &str,
        request_data: &JsonMap,
    ) -> Result<Box<dyn CapacitySlot>, PreCallHookError> {
        let guard = self
            .request_capacity(caller, model, Some(request_data))
            .await?;
        Ok(Box::new(guard))
    }
}

// counter_for_model: one model group's counter, taking that group's capacity around each provider count.
pub struct CapacityCounter<'counter, Host: CacheRoutingHost + ?Sized> {
    pub host: &'counter Host,
    pub caller: &'counter UserAPIKeyAuth,
    pub model: &'counter str,
    pub request_data: &'counter JsonMap,
}

impl<Host: CacheRoutingHost + ?Sized> CapacityCounter<'_, Host> {
    // the capacity slot the count runs under; a limiter that refuses or fails yields no estimate
    pub(crate) async fn capacity(&self) -> Result<Box<dyn CapacitySlot>, CacheAlternativesError> {
        let model = self.model.to_owned();
        let limiter =
            self.host
                .capacity_limiter()
                .ok_or(CacheAlternativesError::CapacityUnavailable {
                    model: model.clone(),
                })?;
        limiter
            .capacity(self.caller, self.model, self.request_data)
            .await
            .map_err(|source| CacheAlternativesError::Capacity { model, source })
    }
}

impl<Host: CacheRoutingHost + ?Sized> TokenCounter for CapacityCounter<'_, Host> {
    async fn count(&self, model: &str, api_key: &SecretString, body: &Json) -> Option<i64> {
        let _slot = self.capacity().await.ok()?;
        let services = self.host.prediction_services();
        AnthropicCacheRouting::count_tokens(
            self.host.count_tokens_handler(),
            services.settings,
            model,
            api_key,
            body,
        )
        .await
    }
}
