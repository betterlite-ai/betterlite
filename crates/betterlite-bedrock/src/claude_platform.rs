pub mod chat;
pub mod convert;
pub mod error;
pub mod native;
pub mod stream;

mod tests;

use betterlite_config::sections::claude_platform_inputs::{PlatformEnvironment, PlatformInputs};
use secrecy::{ExposeSecret as _, SecretString};

use crate::claude_platform_inputs::workspace::resolve_workspace_id;
use crate::model_routes::ParamMap;

// LiteLLM's BedrockClaudePlatformConfig.custom_llm_provider
pub const CUSTOM_LLM_PROVIDER: &str = "bedrock";
// The provider the inherited AnthropicMessagesConfig keeps, which the native Messages config never overrides
pub const MESSAGES_PROVIDER: &str = "anthropic";
// LiteLLM's BedrockClaudePlatformConfig.should_strip_billing_metadata
pub const STRIPS_BILLING_METADATA: bool = true;
// LiteLLM's BedrockClaudePlatformMessagesConfig.should_filter_anthropic_beta_headers
pub const FILTERS_ANTHROPIC_BETAS: bool = false;
// The llm_provider its AuthenticationError carries
pub const AUTHENTICATION_PROVIDER: &str = "bedrock";
pub const WORKSPACE_HEADER: &str = "anthropic-workspace-id";

#[derive(Clone, Copy)]
pub struct WorkspaceSources<'request> {
    pub deployment: &'request PlatformInputs,
    pub global: &'request PlatformInputs,
    pub environment: &'request PlatformEnvironment,
}

// _get_workspace_id returns None only when every alias and both environment values are unset.
pub fn require_workspace(
    model: &str,
    optional_params: &ParamMap,
    workspace: WorkspaceSources<'_>,
) -> Result<String, error::WorkspaceError> {
    let resolved = resolve_workspace_id(
        optional_params,
        workspace.deployment,
        workspace.global,
        workspace.environment,
    )?;
    resolved.ok_or_else(|| error::WorkspaceError::Missing {
        model: model.to_owned(),
    })
}

// `api_key or get_secret_str("ANTHROPIC_AWS_API_KEY")`: Python's `or` drops an empty request key.
#[must_use]
pub fn resolve_api_key(
    requested: Option<&SecretString>,
    environment: &PlatformEnvironment,
) -> Option<SecretString> {
    let from_request = requested.filter(|value| !value.expose_secret().is_empty());
    if let Some(key) = from_request {
        return Some(key.clone());
    }
    environment
        .anthropic_aws_api_key
        .as_ref()
        .filter(|value| !value.expose_secret().is_empty())
        .cloned()
}
