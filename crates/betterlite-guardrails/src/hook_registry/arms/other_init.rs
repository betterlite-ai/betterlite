use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::super::arm::Arms;
use super::super::error::HookRegistryError;
use super::super::kind::HookKind;
use super::super::ports::HookPorts;
use super::super::spec::{HookSettings, HookSpec};
use super::super::wire;
use crate::content_filter_init::initialize_content_filter;
use crate::hook_initializers::agent_365::{IgnoredOverride, initialize_agent_365};
use crate::mcp_jwt_signer_init::initialize_mcp_jwt_signer;

pub(super) fn build(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    let arms = Arms { kind, settings };
    Some(match kind {
        wire::AGENT_365 => agent_365(kind, params, name, settings, ports),
        wire::LITELLM_CONTENT_FILTER => {
            let filter =
                initialize_content_filter(params, Some(name), None, None, ports.router_present);
            arms.take(filter, HookKind::LitellmContentFilter)
        }
        wire::MCP_JWT_SIGNER => mcp_jwt(kind, params, name, settings, ports),
        _other => return None,
    })
}

fn agent_365(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    ports: &HookPorts,
) -> Result<HookSpec, HookRegistryError> {
    let (args, ignored) = initialize_agent_365(params, Some(name), ports.secrets.as_ref())
        .map_err(|error| HookRegistryError::init(kind, error))?;
    if !ignored.is_empty() {
        let fields = ignored
            .iter()
            .map(IgnoredOverride::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            "Microsoft Agent 365 ({name}): ignoring {fields}; evaluations always go to the production Agent 365 endpoint and the agent identity is the caller's key alias"
        );
    }
    Ok(HookSpec::new(settings, HookKind::Agent365(Box::new(args))))
}

fn mcp_jwt(
    kind: &str,
    params: &LitellmParams,
    name: &str,
    settings: &HookSettings,
    ports: &HookPorts,
) -> Result<HookSpec, HookRegistryError> {
    let mut callbacks = ports.callbacks();
    let signer = initialize_mcp_jwt_signer(params, Some(name), &mut callbacks, &ports.slot)
        .map_err(|error| HookRegistryError::init(kind, error))?;
    Ok(HookSpec::new(settings, HookKind::McpJwtSigner(signer)))
}
