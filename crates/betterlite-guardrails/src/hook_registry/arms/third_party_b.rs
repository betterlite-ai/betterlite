use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;
use crate::third_party_init_b::panw::initialize_panw_prisma_airs;
use crate::third_party_init_b::pillar::initialize_pillar;
use crate::third_party_init_b::rubrik::initialize_rubrik;
use crate::third_party_init_b::straiker::initialize_straiker;
use crate::third_party_init_b::typesafe::initialize_typesafe;
use crate::third_party_init_b::vigil_guard::initialize_vigil_guard;
use crate::third_party_init_b::xecguard::initialize_xecguard;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::PANW_PRISMA_AIRS => {
            let result = initialize_panw_prisma_airs(params, Some(name));
            arms.take(result, HookKind::PanwPrismaAirs)
        }
        wire::PILLAR => arms.take(initialize_pillar(params, Some(name)), HookKind::Pillar),
        wire::RUBRIK => arms.take(initialize_rubrik(params, Some(name)), HookKind::Rubrik),
        wire::STRAIKER => arms.take(initialize_straiker(params, Some(name)), HookKind::Straiker),
        wire::TYPESAFE => arms.take(initialize_typesafe(params, Some(name)), HookKind::Typesafe),
        wire::VIGIL_GUARD => arms.take(
            initialize_vigil_guard(params, Some(name)),
            HookKind::VigilGuard,
        ),
        wire::XECGUARD => arms.take(initialize_xecguard(params, Some(name)), HookKind::Xecguard),
        _other => return None,
    })
}
