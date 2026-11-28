use betterlite_anthropic::cache_aware_routing::AnthropicCacheRouting;
use betterlite_anthropic::cache_aware_routing::deployment::PredictionDeployment;
use betterlite_anthropic::cache_aware_routing::deployment::WarmRequest;
use betterlite_auth::auth_limit_inputs::account::account_limits;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_router::cache_cost_routing::CacheAwareChoice;
use betterlite_router::cache_cost_routing::PreRoutingResponse;
use betterlite_router::cache_cost_routing::TierParams;
use betterlite_router::router_predicates::JsonMap;
use futures_util::future::join_all;

use comparison::{compare, output_limits};

use crate::routing::cache_alternatives::candidates::{Candidate, available, candidate};
use crate::routing::cache_alternatives::config::CacheRoutingConfig;
use crate::routing::cache_alternatives::eligibility::eligible_models;
use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::host::CacheRoutingHost;

// select_cached_model's arguments, gathered so the ten steps pass one record instead of ten parameters
pub(crate) struct SelectionRequest<'request, Host: CacheRoutingHost + ?Sized> {
    pub(crate) host: &'request Host,
    pub(crate) config: section::Config,
    pub(crate) routing: CacheRoutingConfig<'request>,
    pub(crate) params_for_model: &'request dyn TierParams,
    pub(crate) response: &'request PreRoutingResponse,
    pub(crate) request_kwargs: &'request JsonMap,
    pub(crate) messages: Option<&'request [Message]>,
    pub(crate) body: &'request Json,
    pub(crate) caller: &'request UserAPIKeyAuth,
    pub(crate) now: Option<f64>,
}

// what steps 1 and 2 settle before any candidate, cache or provider read
struct Gate<'gate> {
    provider: AnthropicCacheRouting,
    caller_key_hash: &'gate str,
    names: Vec<(String, String)>,
    checked_at: f64,
}

// select_cached_model: the ten ordered steps, each one returning None keeps the classified route
pub(crate) async fn select_cached_model<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
) -> Result<Option<CacheAwareChoice>, CacheAlternativesError> {
    let Some(gate) = gate(request)? else {
        return Ok(None);
    };
    let candidates = build_candidates(request, &gate.names)?;
    let Some((original, alternatives)) = split_candidates(&candidates, &request.response.model)
    else {
        return Ok(None);
    };
    let warm = warm_candidates(
        request,
        &gate.provider,
        &alternatives,
        gate.caller_key_hash,
        gate.checked_at,
    )
    .await;
    if warm.is_empty() {
        return Ok(None);
    }
    let Some(authorized) = staying_candidates(request, original, &warm).await else {
        return Ok(None);
    };
    let members = members_of(original, &authorized);
    let Some(limits) = output_limits(request, &members, gate.provider.requested_output_limit)
    else {
        return Ok(None);
    };
    Ok(compare(request, &gate, &members, limits).await)
}

// steps 1 and 2: the setting, the body, the decision, the caller's key and the eligible names
fn gate<'gate, Host: CacheRoutingHost + ?Sized>(
    request: &'gate SelectionRequest<'_, Host>,
) -> Result<Option<Gate<'gate>>, CacheAlternativesError> {
    if !request.config.enabled {
        return Ok(None);
    }
    let Some(provider) = AnthropicCacheRouting::from_body(request.body) else {
        return Ok(None);
    };
    let Some(decision) = request.response.routing_decision.as_ref() else {
        return Ok(None);
    };
    // not caller.api_key: a caller without a key cannot seed an observation lookup
    let Some(caller_key_hash) = account_limits(request.caller)
        .hashed_api_key
        .filter(|key| !key.is_empty())
    else {
        return Ok(None);
    };
    let names = eligible_models(&request.routing, decision);
    if !names
        .iter()
        .any(|(_, model)| model.as_str() == request.response.model.as_str())
    {
        return Ok(None);
    }
    let checked_at = request.now.unwrap_or_else(|| request.host.clock_seconds());
    Ok(Some(Gate {
        provider,
        caller_key_hash,
        names,
        checked_at,
    }))
}

// step 3: the candidates that survived the single-deployment and prediction-support checks
fn build_candidates<Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    names: &[(String, String)],
) -> Result<Vec<Candidate>, CacheAlternativesError> {
    let services = request.host.prediction_services();
    let mut kept = Vec::with_capacity(names.len());
    for (tier, model) in names {
        if let Some(found) =
            candidate(request.host, &services, tier, model, request.request_kwargs)?
        {
            kept.push(found);
        }
    }
    Ok(kept)
}

// the candidate for the classified model, which must exist, and every other one in tier order
fn split_candidates<'candidates>(
    candidates: &'candidates [Candidate],
    model: &str,
) -> Option<(&'candidates Candidate, Vec<&'candidates Candidate>)> {
    let original = candidates
        .iter()
        .find(|candidate| candidate.model.as_str() == model)?;
    let alternatives = candidates
        .iter()
        .filter(|candidate| candidate.model.as_str() != original.model.as_str())
        .collect();
    Some((original, alternatives))
}

// step 4: the alternatives already holding a fresh observation, before any authorization or provider call
async fn warm_candidates<'warm, Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    provider: &AnthropicCacheRouting,
    alternatives: &[&'warm Candidate],
    caller_key_hash: &str,
    checked_at: f64,
) -> Vec<&'warm Candidate> {
    let services = request.host.prediction_services();
    let lookups: Vec<WarmRequest<'_>> = alternatives
        .iter()
        .map(|alternative| WarmRequest {
            deployment: PredictionDeployment::read(
                &alternative.deployment.litellm_params,
                &alternative.deployment.model_info,
            ),
            caller_key_hash,
            now: checked_at,
        })
        .collect();
    let flags = join_all(
        lookups
            .iter()
            .map(|lookup| provider.is_warm(lookup, &services)),
    )
    .await;
    alternatives
        .iter()
        .copied()
        .zip(flags)
        .filter_map(|(alternative, warm)| warm.then_some(alternative))
        .collect()
}

// step 5: the original must be callable and healthy, and only authorized warm alternatives stay
async fn staying_candidates<'staying, Host: CacheRoutingHost + ?Sized>(
    request: &SelectionRequest<'_, Host>,
    original: &'staying Candidate,
    warm: &[&'staying Candidate],
) -> Option<Vec<&'staying Candidate>> {
    let mut members: Vec<&Candidate> = Vec::with_capacity(warm.len() + 1);
    members.push(original);
    members.extend(warm.iter().copied());
    let availability = join_all(members.iter().map(|member| {
        available(
            request.host,
            member,
            request.caller,
            request.request_kwargs,
            request.messages,
        )
    }))
    .await;
    // _available catches its own failures, so a failed lookup is an unavailable candidate, not an error
    let mut flags = availability
        .into_iter()
        .map(|result| result.unwrap_or(false));
    let original_available = flags.next()?;
    let authorized: Vec<&Candidate> = warm
        .iter()
        .copied()
        .zip(flags)
        .filter_map(|(member, ok)| ok.then_some(member))
        .collect();
    (original_available && !authorized.is_empty()).then_some(authorized)
}

fn members_of<'members>(
    original: &'members Candidate,
    authorized: &[&'members Candidate],
) -> Vec<&'members Candidate> {
    let mut members = Vec::with_capacity(authorized.len() + 1);
    members.push(original);
    members.extend(authorized.iter().copied());
    members
}

mod comparison;
