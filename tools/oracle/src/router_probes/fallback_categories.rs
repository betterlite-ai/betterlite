pub mod types;

pub type Input = types::Input;
pub type Output = types::Output;

use betterlite_router::fallback_categories::contracts::{
    ContentPolicyResponse, FallbackChain, FallbackFailure, FallbackSuccessor, FallbackTarget,
    MetadataBucket, OrderExternal, OrderFallbacks, OrdinaryFallbacks, RefusalRequest,
    RefusalResponse, SuccessorRequest, WeightedFailover,
};
use betterlite_router::fallback_categories::decide::decide;
use betterlite_router::fallback_categories::gates::{
    has_content_policy_fallback, has_default_fallbacks, raises_anthropic_refusal,
    raises_content_policy_error, refusal_fallback_available,
};
use betterlite_router::weighted_failover::FailoverInputs;

use self::types::{
    Chain, ChainState, External, Failure, FallbackAvailability, MetadataKey, MetadataOutput,
    Ordinary, Payload, RaiseDecision, Refusal, RefusalAvailability, Successor, Target,
};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

struct Prepared {
    window_targets: Vec<FallbackTarget>,
    policy_targets: Vec<FallbackTarget>,
    ordinary_targets: Vec<FallbackTarget>,
    external_targets: Vec<FallbackTarget>,
    deployment_ids: Vec<String>,
    cooldown_ids: Vec<String>,
}

impl Prepared {
    fn new(input: &Payload) -> Self {
        let ordinary_targets = match &input.ordinary {
            Ordinary::Absent => Vec::new(),
            Ordinary::Standard { chain } | Ordinary::NonStandard { chain, .. } => {
                targets_of(&chain.targets)
            }
        };
        Self {
            window_targets: targets_of(&input.context_window.targets),
            policy_targets: targets_of(&input.content_policy.targets),
            ordinary_targets,
            external_targets: targets_of(&input.order.external_targets),
            deployment_ids: input.weighted_failover.deployment_ids.clone(),
            cooldown_ids: input.weighted_failover.cooldown_ids.clone(),
        }
    }

    fn ordinary<'input>(&'input self, input: &'input Payload) -> OrdinaryFallbacks<'input> {
        let chain = match &input.ordinary {
            Ordinary::Absent => FallbackChain::Absent,
            Ordinary::Standard { chain } | Ordinary::NonStandard { chain, .. } => {
                chain_of(chain, &self.ordinary_targets)
            }
        };
        match &input.ordinary {
            Ordinary::Absent => OrdinaryFallbacks::Absent,
            Ordinary::Standard { .. } => OrdinaryFallbacks::Standard(chain),
            Ordinary::NonStandard { .. } => OrdinaryFallbacks::NonStandard(chain),
        }
    }

    fn request<'input>(&'input self, input: &'input Payload) -> SuccessorRequest<'input> {
        SuccessorRequest {
            failure: match input.failure {
                Failure::ContextWindow => FallbackFailure::ContextWindow,
                Failure::ContentPolicy => FallbackFailure::ContentPolicy,
                Failure::Ordinary => FallbackFailure::Ordinary,
            },
            original_model_group: input.original_model_group.as_deref(),
            disable_fallbacks: input.disable_fallbacks,
            guardrail_intervention: input.guardrail_intervention,
            order: OrderFallbacks {
                model_group: &input.order.model_group,
                deployment_orders: &input.order.deployment_orders,
                current_target: input.order.current_target,
                external: match input.order.external {
                    External::Absent => OrderExternal::Absent,
                    External::Targets => OrderExternal::Targets(&self.external_targets),
                    External::ClientFallbacks => OrderExternal::ClientFallbacks,
                },
            },
            weighted_failover: WeightedFailover {
                enabled: input.weighted_failover.enabled,
                inputs: FailoverInputs {
                    strategy: &input.weighted_failover.strategy,
                    failed_deployment_id: input.weighted_failover.failed_deployment_id.as_deref(),
                    model_group: &input.weighted_failover.model_group,
                    deployment_ids: &self.deployment_ids,
                    cooldown_ids: &self.cooldown_ids,
                },
                metadata_key: match input.weighted_failover.metadata_key {
                    MetadataKey::Metadata => MetadataBucket::Metadata,
                    MetadataKey::LitellmMetadata => MetadataBucket::LitellmMetadata,
                },
                prior_excluded_ids: &input.weighted_failover.prior_excluded_ids,
            },
            context_window: chain_of(&input.context_window, &self.window_targets),
            content_policy: chain_of(&input.content_policy, &self.policy_targets),
            ordinary: self.ordinary(input),
        }
    }
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    validate(payload)?;
    let prepared = Prepared::new(payload);
    let request = prepared.request(payload);
    let ordinary = prepared.ordinary(payload);
    Ok(ProbeOutput::FallbackCategories(decide_output(
        payload, &request, &ordinary,
    )))
}

fn validate(payload: &Payload) -> Result<(), OperationError> {
    // the Python scenario builds the model list from these two arrays, so they describe the same deployments
    if payload.order.deployment_orders.len() != payload.weighted_failover.deployment_ids.len() {
        return Err(OperationError::InvalidRequest {
            message: "deployment_orders and deployment_ids describe the same deployments"
                .to_owned(),
        });
    }
    if !payload.weighted_failover.cooldown_ids.is_empty() {
        return Err(OperationError::InvalidRequest {
            message: "the fallback_categories probe cannot seed cooled-down deployments".to_owned(),
        });
    }
    Ok(())
}

fn decide_output(
    payload: &Payload,
    request: &SuccessorRequest<'_>,
    ordinary: &OrdinaryFallbacks<'_>,
) -> Output {
    let router_fallbacks: Vec<&str> = payload
        .router_fallbacks
        .iter()
        .map(String::as_str)
        .collect();
    let refusal = RefusalRequest {
        fallbacks_disabled: payload.disable_fallbacks,
        content_policy: request.content_policy,
        ordinary,
        router_fallbacks: &router_fallbacks,
    };
    let content_response = ContentPolicyResponse {
        has_choices: payload.response.has_choices,
        first_finish_reason: payload.response.first_finish_reason.as_deref(),
    };
    let decision = decide(request);
    Output {
        successor: successor_output(&decision),
        metadata: metadata_output(payload, &decision),
        fallback: FallbackAvailability {
            default_fallbacks: has_default_fallbacks(&router_fallbacks),
            content_policy: has_content_policy_fallback(&request.content_policy, &router_fallbacks),
        },
        refusal: RefusalAvailability {
            available: refusal_fallback_available(&refusal),
        },
        raise_decision: RaiseDecision {
            content_policy: raises_content_policy_error(
                &content_response,
                &request.content_policy,
                &router_fallbacks,
            ),
            anthropic_refusal: raises_anthropic_refusal(
                &payload.response.function_name,
                refusal_response(payload.response.refusal),
                &refusal,
            ),
        },
    }
}

fn metadata_output(payload: &Payload, decision: &FallbackSuccessor) -> MetadataOutput {
    let mut excluded_deployment_ids = payload.weighted_failover.prior_excluded_ids.clone();
    if let FallbackSuccessor::RunWeightedFailover {
        excluded_deployment_ids: written,
        ..
    } = decision
    {
        excluded_deployment_ids.clone_from(written);
    }
    excluded_deployment_ids.sort_unstable();
    MetadataOutput {
        bucket: payload.weighted_failover.metadata_key,
        excluded_deployment_ids,
    }
}

const fn refusal_response(refusal: Refusal) -> RefusalResponse {
    match refusal {
        Refusal::NotRefusal => RefusalResponse::NotRefusal,
        Refusal::WithoutStopDetails => RefusalResponse::RefusalWithoutStopDetails,
        Refusal::WithStopDetails => RefusalResponse::RefusalWithStopDetails,
    }
}

const fn chain_of<'targets>(
    chain: &Chain,
    targets: &'targets [FallbackTarget],
) -> FallbackChain<'targets> {
    match chain.state {
        ChainState::Absent => FallbackChain::Absent,
        ChainState::Unresolved => FallbackChain::Unresolved,
        ChainState::Resolved => FallbackChain::Resolved {
            targets,
            has_unattempted: chain.has_unattempted,
        },
    }
}

fn targets_of(targets: &[Target]) -> Vec<FallbackTarget> {
    targets
        .iter()
        .map(|target| FallbackTarget {
            model: target.model.clone(),
            target_order: target.target_order,
            excluded_deployment_ids: target.excluded_deployment_ids.clone(),
        })
        .collect()
}

fn successor_output(successor: &FallbackSuccessor) -> Successor {
    match successor {
        FallbackSuccessor::Stop(_) => Successor::Stop,
        FallbackSuccessor::RunClientFallbacks => Successor::Run {
            targets: Vec::new(),
            client_fallbacks_after: true,
        },
        FallbackSuccessor::RunOrder {
            targets,
            client_fallbacks_after,
        } => Successor::Run {
            targets: targets.iter().map(target_output).collect(),
            client_fallbacks_after: *client_fallbacks_after,
        },
        FallbackSuccessor::RunTargets { targets, .. }
        | FallbackSuccessor::RunWeightedFailover { targets, .. } => Successor::Run {
            targets: targets.iter().map(target_output).collect(),
            client_fallbacks_after: false,
        },
    }
}

fn target_output(target: &FallbackTarget) -> Target {
    // the exclusion list is a set in LiteLLM, so the probe compares its members in one order
    let mut excluded_deployment_ids = target.excluded_deployment_ids.clone();
    excluded_deployment_ids.sort_unstable();
    Target {
        model: target.model.clone(),
        target_order: target.target_order,
        excluded_deployment_ids,
    }
}
