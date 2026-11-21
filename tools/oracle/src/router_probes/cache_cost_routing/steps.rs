use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_prompts::message_content::Message;
use betterlite_router::cache_cost_routing::{
    CacheAwareChoice, CacheCostRequest, CacheCostRouting, CachedModelChooser, ChooseCached,
    PreRoutingResponse, TierParams,
};
use betterlite_router::complexity_scoring::tier::ComplexityTier;
use betterlite_router::context_placement::facts::WindowFacts;
use betterlite_router::context_placement::fit::ContextFit;
use betterlite_router::router_predicates::JsonMap;
use rust_decimal::Decimal;
use tokio::time::sleep;

use super::wire::{CallOutput, Input, Output, ResponseInput, ResponseOutput};
use crate::router_probes::error::OperationError;

pub(super) async fn run(input: &Input) -> Result<Output, OperationError> {
    let world = World::assemble(input, fit(input)?, choice(input)?);
    let chooser = ProbeChooser {
        choice: world.choice,
        delay: world.delay,
        calls: AtomicU64::new(0),
        seen: Mutex::new(None),
    };
    let service = CacheCostRouting {
        config: &world.config,
        tier_labels: &world.labels,
        params_for_model: &world.params,
        chooser: &chooser,
    };
    let applied = service
        .apply(CacheCostRequest {
            response: input
                .response
                .as_ref()
                .map(|declared| response(declared, input.messages.clone())),
            messages: input.messages.as_deref(),
            request_kwargs: &input.request_kwargs,
            context_fit: &world.fit,
        })
        .await;
    let calls = chooser.calls.load(Ordering::SeqCst);
    let call = chooser
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(Output {
        calls,
        call,
        response: applied.map(output),
    })
}

struct World {
    config: section::Config,
    labels: BTreeMap<ComplexityTier, String>,
    params: ProbeParams,
    fit: ContextFit,
    choice: Option<CacheAwareChoice>,
    delay: Duration,
}

impl World {
    fn assemble(input: &Input, fit: ContextFit, choice: Option<CacheAwareChoice>) -> Self {
        Self {
            config: config(input),
            labels: labels(&input.tier_labels),
            params: ProbeParams {
                entries: params_map(input),
            },
            fit,
            choice,
            delay: Duration::from_millis(input.chooser_delay_ms),
        }
    }
}

fn config(input: &Input) -> section::Config {
    section::Config {
        enabled: input.enabled,
        output_tokens: input.output_tokens,
        timeout_ms: input.timeout_ms,
    }
}

struct ProbeParams {
    entries: BTreeMap<(String, String), JsonMap>,
}

impl TierParams for ProbeParams {
    fn params_for_model(&self, tier: &str, model: &str) -> JsonMap {
        let key = (tier.to_owned(), model.to_owned());
        self.entries.get(&key).cloned().unwrap_or_default()
    }
}

struct ProbeChooser {
    choice: Option<CacheAwareChoice>,
    delay: Duration,
    calls: AtomicU64,
    seen: Mutex<Option<CallOutput>>,
}

#[async_trait]
impl CachedModelChooser for ProbeChooser {
    async fn choose(&self, request: ChooseCached<'_>) -> Option<CacheAwareChoice> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let seen = CallOutput {
            model: request.response.model.clone(),
            decision_present: request.response.routing_decision.is_some(),
            output_tokens: request.config.output_tokens,
            messages_present: request.messages.is_some(),
            kwargs_keys: request.request_kwargs.keys().cloned().collect(),
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(seen);
        sleep(self.delay).await;
        self.choice.clone()
    }
}

fn labels(declared: &BTreeMap<String, String>) -> BTreeMap<ComplexityTier, String> {
    declared
        .iter()
        .filter_map(|(name, label)| {
            ComplexityTier::built_in(name).map(|tier| (tier, label.clone()))
        })
        .collect()
}

fn params_map(input: &Input) -> BTreeMap<(String, String), JsonMap> {
    input
        .tiers
        .iter()
        .map(|(tier, model)| {
            let params = input
                .params
                .iter()
                .find(|entry| {
                    entry.tier.as_str() == tier.as_str() && entry.model.as_str() == model.as_str()
                })
                .map_or_else(JsonMap::new, |entry| entry.params.clone());
            ((tier.clone(), model.clone()), params)
        })
        .collect()
}

fn fit(input: &Input) -> Result<ContextFit, OperationError> {
    let declared = &input.context_fit;
    let buffer =
        declared
            .buffer
            .parse::<f64>()
            .map_err(|error| OperationError::InvalidRequest {
                message: format!("{} is not a context buffer: {error}", declared.buffer),
            })?;
    Ok(ContextFit {
        facts: declared
            .facts
            .iter()
            .map(|(model, facts)| {
                (
                    model.clone(),
                    WindowFacts {
                        window: facts.window,
                        has_unknown: facts.has_unknown,
                    },
                )
            })
            .collect(),
        needed: declared.needed,
        buffer,
    })
}

fn choice(input: &Input) -> Result<Option<CacheAwareChoice>, OperationError> {
    let Some(declared) = input.choice.as_ref() else {
        return Ok(None);
    };
    Ok(Some(CacheAwareChoice {
        model: declared.model.clone(),
        tier: declared.tier.clone(),
        deployment_id: declared.deployment_id.clone(),
        original_cost: cost(&declared.original_cost)?,
        estimated_cost: cost(&declared.estimated_cost)?,
    }))
}

fn cost(text: &str) -> Result<Decimal, OperationError> {
    text.parse::<Decimal>()
        .map_err(|error| OperationError::InvalidRequest {
            message: format!("{text} is not a decimal cost: {error}"),
        })
}

fn response(declared: &ResponseInput, messages: Option<Vec<Message>>) -> PreRoutingResponse {
    PreRoutingResponse {
        model: declared.model.clone(),
        messages,
        routing_decision: declared.routing_decision.clone(),
        session_affinity_ttl_seconds: declared.session_affinity_ttl_seconds,
        litellm_params: declared.litellm_params.clone(),
    }
}

fn output(applied: PreRoutingResponse) -> ResponseOutput {
    ResponseOutput {
        model: applied.model,
        messages_present: applied.messages.is_some(),
        routing_decision: applied.routing_decision,
        session_affinity_ttl_seconds: applied.session_affinity_ttl_seconds,
        litellm_params: applied.litellm_params,
    }
}
