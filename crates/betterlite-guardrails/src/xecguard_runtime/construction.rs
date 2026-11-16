use std::time::Duration;

use betterlite_config::sections::xecguard_runtime::Settings as RuntimeSettings;

use crate::guardrail_base::GuardrailBase;
use crate::third_party_init_b::xecguard::XecGuardArgs;
use crate::xecguard_primitives::settings::{RawXecGuardSettings, XecGuardSettings};

use super::error::XecGuardRuntimeError;

pub(super) const DEFAULT_API_BASE: &str = "https://api-xecguard.cycraft.ai";
pub(super) const DEFAULT_MODEL: &str = "xecguard_v2";
pub(super) const DEFAULT_GROUNDING_STRICTNESS: &str = "BALANCED";
pub(super) const DEFAULT_BLOCK_ON_ERROR: &str = "true";
pub(super) const DEFAULT_GUARDRAIL_NAME: &str = "xecguard";

pub(super) fn resolve(
    args: &XecGuardArgs,
    settings: &RuntimeSettings,
    base: &GuardrailBase,
) -> Result<XecGuardSettings, XecGuardRuntimeError> {
    let api_key = args
        .api_key
        .clone()
        .or_else(|| settings.api_key.clone())
        .ok_or(XecGuardRuntimeError::MissingApiKey)?;
    let configured_base = args
        .api_base
        .clone()
        .or_else(|| settings.api_base.clone())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    let timeout = base
        .settings
        .timeout
        .map(Duration::try_from_secs_f64)
        .transpose()
        .map_err(|source| XecGuardRuntimeError::UnrepresentableTimeout { source })?;
    Ok(XecGuardSettings::from(RawXecGuardSettings {
        api_key,
        api_base: configured_base.trim_end_matches('/').to_owned(),
        model: args
            .xecguard_model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        policy_names: args.policy_names.clone().filter(|names| !names.is_empty()),
        block_on_error: resolve_block_on_error(args, settings),
        grounding_strictness: args
            .grounding_strictness
            .clone()
            .unwrap_or_else(|| DEFAULT_GROUNDING_STRICTNESS.to_owned()),
        timeout,
    }))
}

fn resolve_block_on_error(args: &XecGuardArgs, settings: &RuntimeSettings) -> bool {
    let Some(block) = args.block_on_error else {
        // LiteLLM reads a true flag from these three words, in any case
        let text = settings
            .block_on_error
            .as_deref()
            .unwrap_or(DEFAULT_BLOCK_ON_ERROR)
            .to_lowercase();
        return text == "true" || text == "1" || text == "yes";
    };
    block
}

pub(super) fn guardrail_name(base: &GuardrailBase) -> String {
    base.settings
        .guardrail_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_GUARDRAIL_NAME)
        .to_owned()
}
