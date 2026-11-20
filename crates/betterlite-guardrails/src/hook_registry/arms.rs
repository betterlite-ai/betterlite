mod classes;
mod deferred;
mod hook_init;
mod other_init;
mod ported;
mod third_party_a;
mod third_party_b;
mod vendor_init;

use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::error::HookRegistryError;
use super::ports::HookPorts;
use super::spec::{HookSettings, HookSpec};

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    guardrail_name: &str,
    settings: &HookSettings,
    ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    deferred::build(kind)
        .or_else(|| third_party_a::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| third_party_b::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| hook_init::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| other_init::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| vendor_init::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| classes::build(kind, params, guardrail_name, settings, ports))
        .or_else(|| ported::build(kind, params, guardrail_name, settings, ports))
}
