use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;
use crate::third_party_init_a::aim::initialize_aim;
use crate::third_party_init_a::akto::initialize_akto;
use crate::third_party_init_a::aporia::initialize_aporia;
use crate::third_party_init_a::cisco::initialize_cisco;
use crate::third_party_init_a::conduct::initialize_conduct;
use crate::third_party_init_a::crowdstrike::initialize_crowdstrike;
use crate::third_party_init_a::deepkeep::initialize_deepkeep;
use crate::third_party_init_a::generic_api::initialize_generic_api;
use crate::third_party_init_a::grayswan::initialize_grayswan;
use crate::third_party_init_a::guardrails_ai::initialize_guardrails_ai;
use crate::third_party_init_a::headroom::initialize_headroom;
use crate::third_party_init_a::lasso::initialize_lasso_folder;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::AIM => arms.take(initialize_aim(params, Some(name)), HookKind::Aim),
        wire::AKTO => arms.take(initialize_akto(params, Some(name)), HookKind::Akto),
        wire::APORIA => arms.take(initialize_aporia(params, Some(name)), HookKind::Aporia),
        wire::CISCO_AI_DEFENSE => {
            let result = initialize_cisco(params, Some(name));
            arms.take(result, HookKind::CiscoAiDefense)
        }
        wire::CONDUCT => arms.take(initialize_conduct(params, Some(name)), HookKind::Conduct),
        wire::CROWDSTRIKE_AIDR => {
            let result = initialize_crowdstrike(params, Some(name));
            arms.take(result, HookKind::CrowdstrikeAidr)
        }
        wire::DEEP_KEEP => arms.take(initialize_deepkeep(params, Some(name)), HookKind::Deepkeep),
        wire::GENERIC_GUARDRAIL_API => {
            let result = initialize_generic_api(params, Some(name));
            arms.take(result, HookKind::GenericGuardrailApi)
        }
        wire::GRAYSWAN => arms.take(initialize_grayswan(params, Some(name)), HookKind::Grayswan),
        wire::GUARDRAILS_AI => {
            let result = initialize_guardrails_ai(params, Some(name));
            arms.take(result, HookKind::GuardrailsAi)
        }
        wire::HEADROOM => arms.take(initialize_headroom(params, Some(name)), HookKind::Headroom),
        wire::LASSO => arms.take(initialize_lasso_folder(params, Some(name)), HookKind::Lasso),
        _other => return None,
    })
}
