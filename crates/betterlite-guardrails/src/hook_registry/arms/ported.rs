use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::kinds::{
    initialize_alice, initialize_cato_networks, initialize_dynamoai, initialize_enkryptai,
};
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    _ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::ALICE => Ok(HookSpec::new(
            settings,
            HookKind::Alice(Box::new(initialize_alice(params, Some(name)))),
        )),
        wire::CATO_NETWORKS => {
            let result = initialize_cato_networks(params, Some(name));
            arms.take(result, HookKind::CatoNetworks)
        }
        wire::DYNAMOAI => Ok(HookSpec::new(
            settings,
            HookKind::Dynamoai(Box::new(initialize_dynamoai(params, Some(name)))),
        )),
        wire::ENKRYPTAI => arms.take(
            initialize_enkryptai(params, Some(name)),
            HookKind::Enkryptai,
        ),
        _other => return None,
    })
}
