use serde::Serialize;

use crate::fallback_categories::contracts::{
    FallbackChain, FallbackFailure, FallbackSuccessor, FallbackTarget, MetadataBucket,
    OrderExternal, OrdinaryFallbacks, StopReason, SuccessorRequest, TargetSource, WeightedFailover,
};
use crate::weighted_failover::{FailoverEntry, plan_weighted_failover};

// the weighted-failover planner reads and writes this bucket of LiteLLM's request map
#[derive(Serialize)]
struct FailoverRequest<'state> {
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<FailoverMetadata<'state>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    litellm_metadata: Option<FailoverMetadata<'state>>,
}

#[derive(Serialize)]
struct FailoverMetadata<'state> {
    _failover_excluded_ids: &'state [String],
}

#[must_use]
pub fn decide(request: &SuccessorRequest<'_>) -> FallbackSuccessor {
    if let Some(reason) = early_stop(request) {
        return FallbackSuccessor::Stop(reason);
    }
    if !request.failure.skips_order_fallback() {
        if let Some(successor) = order_successor(request) {
            return successor;
        }
        if let Some(entry) = weighted_failover_entry(request) {
            let excluded_deployment_ids = entry.excluded_deployment_ids.clone();
            return FallbackSuccessor::RunWeightedFailover {
                targets: vec![FallbackTarget::from(entry)],
                excluded_deployment_ids,
            };
        }
    }
    if matches!(request.ordinary, OrdinaryFallbacks::NonStandard(_)) {
        return FallbackSuccessor::RunClientFallbacks;
    }
    if let Some(successor) = category_successor(request) {
        return successor;
    }
    ordinary_successor(&request.ordinary)
        .unwrap_or(FallbackSuccessor::Stop(StopReason::NoFallbackChain))
}

const fn early_stop(request: &SuccessorRequest<'_>) -> Option<StopReason> {
    if request.disable_fallbacks {
        return Some(StopReason::FallbacksDisabled);
    }
    if request.original_model_group.is_none() {
        return Some(StopReason::MissingModelGroup);
    }
    if request.guardrail_intervention {
        return Some(StopReason::GuardrailIntervention);
    }
    None
}

fn order_successor(request: &SuccessorRequest<'_>) -> Option<FallbackSuccessor> {
    let order = &request.order;
    let mut levels: Vec<i64> = order.deployment_orders.iter().flatten().copied().collect();
    levels.sort_unstable();
    levels.dedup();
    if levels.len() < 2 {
        return None;
    }
    let skip_up_to = order.current_target.unwrap_or(*levels.first()?);
    let mut targets: Vec<FallbackTarget> = levels
        .iter()
        .filter(|level| **level > skip_up_to)
        .map(|level| FallbackTarget {
            model: order.model_group.to_owned(),
            target_order: Some(*level),
            excluded_deployment_ids: Vec::new(),
        })
        .collect();
    let client_fallbacks_after = match order.external {
        OrderExternal::Absent => false,
        OrderExternal::Targets(external) => {
            targets.extend_from_slice(external);
            false
        }
        OrderExternal::ClientFallbacks => true,
    };
    if targets.is_empty() && !client_fallbacks_after {
        return None;
    }
    Some(FallbackSuccessor::RunOrder {
        targets,
        client_fallbacks_after,
    })
}

fn weighted_failover_entry(request: &SuccessorRequest<'_>) -> Option<FailoverEntry> {
    if !request.weighted_failover.enabled {
        return None;
    }
    plan_failover(&request.weighted_failover)
}

fn plan_failover(request: &WeightedFailover<'_>) -> Option<FailoverEntry> {
    let metadata = FailoverMetadata {
        _failover_excluded_ids: request.prior_excluded_ids,
    };
    let call = match request.metadata_key {
        MetadataBucket::Metadata => FailoverRequest {
            metadata: Some(metadata),
            litellm_metadata: None,
        },
        MetadataBucket::LitellmMetadata => FailoverRequest {
            metadata: None,
            litellm_metadata: Some(metadata),
        },
    };
    let mut serialized = serde_json::to_value(&call).ok()?;
    plan_weighted_failover(
        &request.inputs,
        serialized.as_object_mut()?,
        request.metadata_key.key(),
    )
}

fn category_successor(request: &SuccessorRequest<'_>) -> Option<FallbackSuccessor> {
    let (chain, source, missing) = match request.failure {
        FallbackFailure::ContextWindow => (
            request.context_window,
            TargetSource::ContextWindow,
            StopReason::NoContextWindowChain,
        ),
        FallbackFailure::ContentPolicy => (
            request.content_policy,
            TargetSource::ContentPolicy,
            StopReason::NoContentPolicyChain,
        ),
        FallbackFailure::Ordinary => return None,
    };
    match chain {
        FallbackChain::Absent => None,
        FallbackChain::Unresolved => Some(FallbackSuccessor::Stop(missing)),
        FallbackChain::Resolved { targets, .. } => Some(FallbackSuccessor::RunTargets {
            source,
            targets: targets.to_vec(),
        }),
    }
}

fn ordinary_successor(ordinary: &OrdinaryFallbacks<'_>) -> Option<FallbackSuccessor> {
    match ordinary {
        OrdinaryFallbacks::Standard(FallbackChain::Resolved { targets, .. }) => {
            Some(FallbackSuccessor::RunTargets {
                source: TargetSource::Ordinary,
                targets: targets.to_vec(),
            })
        }
        OrdinaryFallbacks::Absent
        | OrdinaryFallbacks::NonStandard(_)
        | OrdinaryFallbacks::Standard(FallbackChain::Absent | FallbackChain::Unresolved) => None,
    }
}
