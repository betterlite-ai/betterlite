use betterlite_router::cooldown_decisions::decision::{
    CooldownDecision, CooldownRequest, DeploymentFacts, MinuteSamples, decide,
};
use betterlite_router::cooldown_decisions::failure::{FailureFacts, Seconds};
use betterlite_router::cooldown_decisions::policy::{
    AllowedFailsPolicy, CooldownPolicy, FailureThresholds,
};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(pub Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub router: RouterInput,
    #[serde(default)]
    pub deployment: Option<DeploymentFacts>,
    pub failure: FailureFacts,
    pub minute: MinuteSamples,
    pub allowed_fails_before: u64,
    #[serde(default)]
    pub time_to_cooldown: Option<Seconds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterInput {
    pub allowed_fails: u32,
    pub default_allowed_fails: u32,
    pub cooldown_time: Seconds,
    pub disable_cooldowns: bool,
    #[serde(default)]
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
}

pub type Output = CooldownDecision;

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    // a deployment the catalog knows always belongs to at least one model group entry
    if payload
        .deployment
        .as_ref()
        .is_some_and(|deployment| deployment.model_group_size == 0)
    {
        return Err(OperationError::InvalidRequest {
            message: "a known deployment has at least one model group entry".to_owned(),
        });
    }
    // LiteLLM's constants carry these defaults and the cases never vary them, so the adapter keeps the pinned values
    let policy = CooldownPolicy {
        allowed_fails: payload.router.allowed_fails,
        default_allowed_fails: payload.router.default_allowed_fails,
        cooldown_time: payload.router.cooldown_time,
        disable_cooldowns: payload.router.disable_cooldowns,
        allowed_fails_policy: payload.router.allowed_fails_policy.clone(),
        thresholds: FailureThresholds::default(),
    };
    let request = CooldownRequest {
        policy: &policy,
        deployment: payload.deployment.as_ref(),
        failure: &payload.failure,
        minute: payload.minute,
        allowed_fails_before: payload.allowed_fails_before,
        time_to_cooldown: payload.time_to_cooldown,
    };
    Ok(ProbeOutput::CooldownDecisions(decide(&request)))
}
