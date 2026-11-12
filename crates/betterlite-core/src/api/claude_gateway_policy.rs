use betterlite_config::sections::claude_gateway_policy::{
    ManagedSettings, Settings as ResolvedSettings,
};
use serde::Deserialize;
use thiserror::Error;

#[path = "claude_gateway_policy/tests.rs"]
mod tests;

pub const DISABLED_STATUS_CODE: u16 = 404;
pub const DISABLED_DETAIL: &str = "Claude Code gateway is not enabled";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeGatewayPolicyInput {
    pub general_settings: GatewayGeneralSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayGeneralSettings {
    #[serde(default)]
    pub enable_claude_code_gateway: Option<bool>,
    #[serde(default)]
    pub managed_settings: Option<ManagedSettings>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaudeGatewayPolicyResult<'settings> {
    pub enabled: bool,
    pub managed_settings: Option<&'settings ManagedSettings>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClaudeGatewayPolicyError {
    #[error("{detail}")]
    Disabled {
        status_code: u16,
        detail: &'static str,
    },
}

impl From<&ResolvedSettings> for ClaudeGatewayPolicyInput {
    fn from(settings: &ResolvedSettings) -> Self {
        Self {
            general_settings: GatewayGeneralSettings {
                enable_claude_code_gateway: Some(settings.enabled),
                managed_settings: settings.managed_settings.clone(),
            },
        }
    }
}

#[must_use]
pub fn evaluate(input: &ClaudeGatewayPolicyInput) -> ClaudeGatewayPolicyResult<'_> {
    ClaudeGatewayPolicyResult {
        enabled: input
            .general_settings
            .enable_claude_code_gateway
            .unwrap_or(false),
        managed_settings: input.general_settings.managed_settings.as_ref(),
    }
}

pub fn ensure_gateway_enabled(
    input: &ClaudeGatewayPolicyInput,
) -> Result<ClaudeGatewayPolicyResult<'_>, ClaudeGatewayPolicyError> {
    let result = evaluate(input);
    if result.enabled {
        return Ok(result);
    }
    Err(ClaudeGatewayPolicyError::Disabled {
        status_code: DISABLED_STATUS_CODE,
        detail: DISABLED_DETAIL,
    })
}
