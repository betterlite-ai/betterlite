use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::cache_aware_routing::AnthropicCacheRouting;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use betterlite_router::cache_cost_routing::CacheAwareChoice;
use betterlite_router::cache_cost_routing::CachedModelChooser;
use betterlite_router::cache_cost_routing::ChooseCached;
use betterlite_router::prompt_affinity::json::messages_json;
use tokio::time::timeout;

use crate::routing::cache_alternatives::CacheAlternatives;
use crate::routing::cache_alternatives::eligibility::decision_text;
use crate::routing::cache_alternatives::eligibility::eligible_models;
use crate::routing::cache_alternatives::eligibility::supported_router_marker;
use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::host::CacheRoutingHost;
use crate::routing::cache_alternatives::request::ProxyRequest;
use crate::routing::cache_alternatives::request::caller;
use crate::routing::cache_alternatives::request::refused_request_fields;
use crate::routing::cache_alternatives::selection::SelectionRequest;
use crate::routing::cache_alternatives::selection::select_cached_model;

const ROUTER_MODEL_NAME: &str = "router_model_name";

#[async_trait]
impl<Host: CacheRoutingHost> CachedModelChooser for CacheAlternatives<Host> {
    async fn choose(&self, request: ChooseCached<'_>) -> Option<CacheAwareChoice> {
        choose_cached_model(&self.host, &request).await
    }
}

// choose_cached_model: the configured deadline over the whole comparison, and every failure keeps the classified model
pub async fn choose_cached_model<Host>(
    host: &Host,
    request: &ChooseCached<'_>,
) -> Option<CacheAwareChoice>
where
    Host: CacheRoutingHost + ?Sized,
{
    if !request.config.enabled {
        return None;
    }
    match timeout(
        Duration::from_millis(request.config.timeout_ms),
        choose_within(host, request),
    )
    .await
    {
        Ok(Ok(Some(choice))) => Some(choice),
        Ok(Ok(None)) => None,
        Ok(Err(_)) | Err(_) => {
            tracing::debug!("Cache-aware routing unavailable; keeping the classified model");
            None
        }
    }
}

// _choose_cached_model: the proxy-state gates, then the request rebuild and the ordered selection
async fn choose_within<Host>(
    host: &Host,
    request: &ChooseCached<'_>,
) -> Result<Option<CacheAwareChoice>, CacheAlternativesError>
where
    Host: CacheRoutingHost + ?Sized,
{
    let Some(decision) = request.response.routing_decision.as_ref() else {
        return Ok(None);
    };
    let routing = host.routing_config();
    if eligible_models(&routing, decision).is_empty() {
        return Ok(None);
    }
    if proxy_refuses(host, request) {
        return Ok(None);
    }
    let alias = decision_text(decision, ROUTER_MODEL_NAME).unwrap_or_default();
    if !supported_router_marker(host, alias, request.request_kwargs)? {
        return Ok(None);
    }
    let Some((api_key_auth, body)) = prepared_request(host, request)? else {
        return Ok(None);
    };
    select_cached_model(&SelectionRequest {
        host,
        config: request.config,
        routing,
        params_for_model: request.params_for_model,
        response: request.response,
        request_kwargs: request.request_kwargs,
        messages: request.messages,
        body: &body,
        caller: &api_key_auth,
        now: None,
    })
    .await
}

// the proxy state that keeps the comparison out: another router, plugins, transforms or a pending compaction
fn proxy_refuses<Host>(host: &Host, request: &ChooseCached<'_>) -> bool
where
    Host: CacheRoutingHost + ?Sized,
{
    !host.proxy_uses_this_router()
        || host.has_routing_plugins()
        || host.has_request_transforms()
        || host.compaction_pending(request.request_kwargs)
}

// the caller, the strict proxy request, the refused request fields, the rebuilt body and the limiter
fn prepared_request<Host>(
    host: &Host,
    request: &ChooseCached<'_>,
) -> Result<Option<(UserAPIKeyAuth, Json)>, CacheAlternativesError>
where
    Host: CacheRoutingHost + ?Sized,
{
    let Some(api_key_auth) = caller(request.request_kwargs)? else {
        return Ok(None);
    };
    let Some(proxy_request) = ProxyRequest::from_kwargs(request.request_kwargs) else {
        return Ok(None);
    };
    if refused_request_fields(request.request_kwargs) {
        return Ok(None);
    }
    let messages = request
        .messages
        .map(|request_messages| Json::List(messages_json(request_messages)));
    let Some(body) = AnthropicCacheRouting::request_body(
        &proxy_request.url,
        &proxy_request.headers,
        &proxy_request.body,
        request.request_kwargs,
        messages.as_ref(),
    ) else {
        return Ok(None);
    };
    if host.capacity_limiter().is_none() {
        return Ok(None);
    }
    Ok(Some((api_key_auth, body)))
}
