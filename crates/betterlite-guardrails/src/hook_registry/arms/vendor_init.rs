use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;
use crate::promptguard::init::initialize_promptguard;
use crate::qohash::init::initialize_qohash;
use crate::qualifire::init::initialize_qualifire;
use crate::semantic_guard_hook::init::initialize_semantic_guard;
use crate::singulr::init::initialize_singulr;
use crate::zscaler_ai_guard::init::initialize_zscaler;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::PROMPTGUARD => {
            let result = initialize_promptguard(params, Some(name));
            arms.take(result, HookKind::Promptguard)
        }
        wire::QOSTODIAN_NEXUS => Ok(arms.plain(
            initialize_qohash(params, Some(name)),
            HookKind::QostodianNexus,
        )),
        wire::QUALIFIRE => {
            let result = initialize_qualifire(params, Some(name));
            arms.take(result, HookKind::Qualifire)
        }
        wire::SEMANTIC_GUARD => {
            let result = initialize_semantic_guard(
                params,
                Some(name),
                ports.router_present,
                ports.environment.as_ref(),
            );
            arms.take(result, HookKind::SemanticGuard)
        }
        wire::SINGULR => arms.take(initialize_singulr(params, Some(name)), HookKind::Singulr),
        wire::ZSCALER_AI_GUARD => {
            let result = initialize_zscaler(params, Some(name));
            arms.take(result, HookKind::ZscalerAiGuard)
        }
        _other => return None,
    })
}
