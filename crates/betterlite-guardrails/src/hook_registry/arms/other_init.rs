use betterlite_config::sections::guardrail_params::params::LitellmParams;

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
    Some(match kind {
        wire::AGENT_365 => match initialize_agent_365(params, Some(name), ports.secrets.as_ref()) {
            Ok((args, ignored)) => {
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
            Err(error) => Err(HookRegistryError::init(kind, error)),
        },
        wire::LITELLM_CONTENT_FILTER => {
            match initialize_content_filter(params, Some(name), None, None, ports.router_present) {
                Ok(args) => Ok(HookSpec::new(
                    settings,
                    HookKind::LitellmContentFilter(Box::new(args)),
                )),
                Err(error) => Err(HookRegistryError::init(kind, error)),
            }
        }
        wire::MCP_JWT_SIGNER => {
            let mut callbacks = ports.callbacks();
            match initialize_mcp_jwt_signer(params, Some(name), &mut callbacks, &ports.slot) {
                Ok(signer) => Ok(HookSpec::new(settings, HookKind::McpJwtSigner(signer))),
                Err(error) => Err(HookRegistryError::init(kind, error)),
            }
        }
        _other => return None,
    })
}
