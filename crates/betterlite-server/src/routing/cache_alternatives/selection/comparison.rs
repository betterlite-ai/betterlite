use betterlite_anthropic::cache_aware_routing::AnthropicCacheRouting;
use betterlite_anthropic::cache_aware_routing::deployment::ArmRequest;
use betterlite_anthropic::cache_aware_routing::deployment::PredictionDeployment;
use betterlite_anthropic::cache_aware_routing::services::PredictionServices;
use betterlite_anthropic::cache_aware_routing::types::CachePredictionArm;
use betterlite_anthropic::cache_aware_routing::types::CacheState;
use betterlite_models::decisions::Json;
use betterlite_router::cache_cost_routing::CacheAwareChoice;
use betterlite_router::cache_cost_routing::TierParams;
use futures_util::future::join_all;
use rust_decimal::Decimal;

use super::Gate;
use super::SelectionRequest;
use crate::routing::cache_alternatives::candidates::Candidate;
use crate::routing::cache_alternatives::host::CacheRoutingHost;
use crate::routing::cache_alternatives::host::CapacityCounter;

// one compared arm with the output limit its own parameters gave it
struct Compared<'compared> {
    candidate: &'compared Candidate,
    limit: i64,
    arm: CachePredictionArm,
}

// steps 6 to 10: the limits, the predictions under capacity, the strict and fresh filter, the cheapest arm
pub(super) async fn compare<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    gate: &Gate<'_>,
    members: &[&Candidate],
    limits: Vec<i64>,
) -> Option<CacheAwareChoice> {
    let pairs: Vec<(&Candidate, i64)> = members.iter().copied().zip(limits).collect();
    let arms = predict_arms(
        request,
        &gate.provider,
        &pairs,
        gate.caller_key_hash,
        gate.checked_at,
    )
    .await;
    let compared: Vec<Compared<'_>> = pairs
        .into_iter()
        .zip(arms)
        .map(|((candidate, limit), arm)| Compared {
            candidate,
            limit,
            arm,
        })
        .collect();
    let services = request.host.prediction_services();
    pick_choice(request, &gate.provider, &services, &compared)
}

// step 6: each compared model's max_tokens, else the caller's requested limit, and a bad limit stops the comparison
pub(super) fn output_limits<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    members: &[&Candidate],
    fallback: i64,
) -> Option<Vec<i64>> {
    let mut limits = Vec::with_capacity(members.len());
    for member in members {
        let params = request
            .params_for_model
            .params_for_model(&member.tier, &member.model);
        let limit = match params.get("max_tokens") {
            Some(value) => positive_int(value)?,
            None => fallback,
        };
        limits.push(limit);
    }
    Some(limits)
}

fn positive_int(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64().filter(|limit| *limit > 0),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// step 7: one prediction per compared model, each count running under that model group's capacity
async fn predict_arms<'predict, Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    provider: &AnthropicCacheRouting,
    compared: &[(&'predict Candidate, i64)],
    caller_key_hash: &str,
    checked_at: f64,
) -> Vec<CachePredictionArm> {
    let services = request.host.prediction_services();
    let counters: Vec<CapacityCounter<'_, Host>> = compared
        .iter()
        .map(|(candidate, _)| CapacityCounter {
            host: request.host,
            caller: request.caller,
            model: candidate.model.as_str(),
            request_data: request.request_kwargs,
        })
        .collect();
    let arm_requests: Vec<ArmRequest<'_>> = compared
        .iter()
        .map(|(candidate, _)| ArmRequest {
            deployment: PredictionDeployment::read(
                &candidate.deployment.litellm_params,
                &candidate.deployment.model_info,
            ),
            caller_key_hash,
            checked_at,
        })
        .collect();
    join_all(
        arm_requests
            .iter()
            .zip(counters.iter())
            .map(|(arm_request, counter)| provider.predict(arm_request, counter, &services)),
    )
    .await
}

// steps 8 to 10: the capped original cost, the strict and fresh alternatives, and the cheapest of them
fn pick_choice<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    provider: &AnthropicCacheRouting,
    services: &PredictionServices<'_>,
    compared: &[Compared<'_>],
) -> Option<CacheAwareChoice> {
    let (original, alternatives) = compared.split_first()?;
    let original_cost = cost_of(request, services, original)?;
    let finished_at = request.now.unwrap_or_else(|| request.host.clock_seconds());
    let mut best: Option<CacheAwareChoice> = None;
    for alternative in alternatives {
        let Some(cost) = cost_of(request, services, alternative) else {
            continue;
        };
        let fresh = matches!(
            alternative.arm.cache_state,
            CacheState::Warm | CacheState::Partial
        ) && alternative
            .arm
            .evidence
            .as_ref()
            .is_some_and(|evidence| evidence.expires_at > finished_at)
            && alternative.arm.estimate.as_ref().is_some_and(|estimate| {
                AnthropicCacheRouting::fits(
                    &PredictionDeployment::read(
                        &alternative.candidate.deployment.litellm_params,
                        &alternative.candidate.deployment.model_info,
                    ),
                    estimate.tokens.total_tokens(),
                    alternative.limit,
                    services,
                    request.host.price_lookup(),
                )
            });
        if cost >= original_cost || !fresh {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| cost < current.estimated_cost)
        {
            best = Some(CacheAwareChoice {
                model: alternative.candidate.model.clone(),
                tier: alternative.candidate.tier.clone(),
                deployment_id: alternative.arm.deployment_id.clone(),
                original_cost,
                estimated_cost: cost,
            });
        }
    }
    best
}

// min(cache_aware_routing_output_tokens, limit): the count a price is asked for caps the caller's output limit
fn cost_of<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    services: &PredictionServices<'_>,
    compared: &Compared<'_>,
) -> Option<Decimal> {
    let capped = i64::try_from(request.config.output_tokens)
        .unwrap_or(i64::MAX)
        .min(compared.limit);
    AnthropicCacheRouting::cost(services.cost, &compared.arm, capped, services.billing_time)
}
