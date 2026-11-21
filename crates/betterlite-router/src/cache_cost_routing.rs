use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_redact::structured::mask_credentials_in_payload;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use tokio::time::timeout;

use crate::complexity_scoring::decision::RoutingDecision;
use crate::complexity_scoring::tier::ComplexityTier;
use crate::context_placement::fit::ContextFit;
use crate::router_predicates::JsonMap;

mod tests;

// RoutingDecisionCause::PromptCacheCost: the cause a decision the cache comparison replaced reports
const CAUSE: &str = "prompt_cache_cost";
const COST_PLACES: u32 = 8;

// CacheAwareChoice: the warm same-or-higher-tier model a cache prediction picked, with both compared costs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheAwareChoice {
    pub model: String,
    pub tier: String,
    pub deployment_id: String,
    pub original_cost: Decimal,
    pub estimated_cost: Decimal,
}

// PreRoutingHookResponse: the classified route a cache comparison may replace
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreRoutingResponse {
    pub model: String,
    pub messages: Option<Vec<Message>>,
    pub routing_decision: Option<RoutingDecision>,
    pub session_affinity_ttl_seconds: Option<u32>,
    pub litellm_params: Option<JsonMap>,
}

// _litellm_params_for_model as the callable choose_cached_model receives: a tier entry's params for one model
pub trait TierParams: Send + Sync {
    fn params_for_model(&self, tier: &str, model: &str) -> JsonMap;
}

// choose_cached_model's arguments besides the router it holds
pub struct ChooseCached<'request> {
    pub config: section::Config,
    pub params_for_model: &'request dyn TierParams,
    pub response: &'request PreRoutingResponse,
    pub request_kwargs: &'request JsonMap,
    pub messages: Option<&'request [Message]>,
}

// choose_cached_model: cache prediction and candidate eligibility; None keeps the classified route
#[async_trait]
pub trait CachedModelChooser: Send + Sync {
    async fn choose(&self, request: ChooseCached<'_>) -> Option<CacheAwareChoice>;
}

pub struct CacheCostRequest<'request> {
    pub response: Option<PreRoutingResponse>,
    pub messages: Option<&'request [Message]>,
    pub request_kwargs: &'request JsonMap,
    pub context_fit: &'request ContextFit,
}

// _apply_prompt_cache_routing: the cache-aware settings, the tier label map, the tier lookup and the prediction it calls
pub struct CacheCostRouting<'deps> {
    pub config: &'deps section::Config,
    pub tier_labels: &'deps BTreeMap<ComplexityTier, String>,
    pub params_for_model: &'deps dyn TierParams,
    pub chooser: &'deps dyn CachedModelChooser,
}

impl CacheCostRouting<'_> {
    // _apply_prompt_cache_routing: a chosen warm model replaces the classified route, and every other outcome keeps it
    pub async fn apply(&self, request: CacheCostRequest<'_>) -> Option<PreRoutingResponse> {
        let Some(response) = request.response else {
            return None;
        };
        if !self.config.enabled || response.routing_decision.is_none() {
            return Some(response);
        }
        let cached = ChooseCached {
            config: *self.config,
            params_for_model: self.params_for_model,
            response: &response,
            request_kwargs: request.request_kwargs,
            messages: request.messages,
        };
        // choose_cached_model bounds the prediction: an expired deadline preserves the classified route
        let deadline = Duration::from_millis(self.config.timeout_ms);
        let choice = timeout(deadline, self.chooser.choose(cached))
            .await
            .ok()
            .flatten();
        let Some(choice) = choice else {
            return Some(response);
        };
        if !request.context_fit.accepts(&choice.model) {
            return Some(response);
        }
        Some(self.stamp(response, &choice))
    }

    fn stamp(
        &self,
        mut response: PreRoutingResponse,
        choice: &CacheAwareChoice,
    ) -> PreRoutingResponse {
        let Some(mut decision) = response.routing_decision.take() else {
            return response;
        };
        let classified_model = response.model.clone();
        let previous_cause = cause_text(decision.get("cause"));
        let mut signals = match decision.get("signals") {
            Some(Json::List(signals)) => signals.clone(),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            ) => Vec::new(),
        };
        signals.push(Json::Text(format!(
            "cache-aware:classified-model={classified_model}"
        )));
        signals.push(Json::Text(format!(
            "cache-aware:classification-cause={previous_cause}"
        )));
        signals.push(Json::Text(format!(
            "cache-aware:estimated-cost={};original-cost={}",
            cost_text(choice.estimated_cost),
            cost_text(choice.original_cost),
        )));
        let params = self
            .params_for_model
            .params_for_model(&choice.tier, &choice.model);
        decision.insert("routed_model".to_owned(), Json::Text(choice.model.clone()));
        decision.insert("cause".to_owned(), Json::Text(CAUSE.to_owned()));
        decision.insert("tier".to_owned(), Json::Text(choice.tier.clone()));
        decision.insert(
            "tier_label".to_owned(),
            Json::Text(self.tier_label(&choice.tier)),
        );
        decision.insert("tier_litellm_params".to_owned(), masked_params(&params));
        decision.insert("signals".to_owned(), Json::List(signals));
        tracing::info!(
            "ComplexityRouter: cache-aware choice model={} original={} estimated_cost={} original_cost={}",
            choice.model,
            classified_model,
            choice.estimated_cost,
            choice.original_cost,
        );
        response.model = choice.model.clone();
        response.litellm_params = Some(params);
        response.routing_decision = Some(decision);
        response
    }

    // tier_labels[tier] or the tier's own name; the stamp writes the label even when it equals the name
    fn tier_label(&self, tier: &str) -> String {
        ComplexityTier::built_in(tier)
            .and_then(|built_in| self.tier_labels.get(&built_in))
            .map_or_else(|| tier.to_owned(), Clone::clone)
    }
}

// routing_decision["cause"] as the signal writes it: an absent or null cause reads as Python's None
fn cause_text(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(text)) => text.clone(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) | Json::List(_)) => {
            "None".to_owned()
        }
    }
}

// :.8f over an exact decimal: round to the eighth place, then write all eight digits
fn cost_text(value: Decimal) -> String {
    format!(
        "{:.8}",
        value.round_dp_with_strategy(COST_PLACES, RoundingStrategy::MidpointNearestEven)
    )
}

// LiteLLM puts the tier params in the spend-log decision unmasked; a tier entry can carry an api_key
fn masked_params(params: &JsonMap) -> Json {
    let Ok(value) = serde_json::to_value(params) else {
        return Json::Object(JsonMap::new());
    };
    match serde_json::from_value(mask_credentials_in_payload(&value)) {
        Ok(masked @ Json::Object(_)) => masked,
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => Json::Object(JsonMap::new()),
    }
}
