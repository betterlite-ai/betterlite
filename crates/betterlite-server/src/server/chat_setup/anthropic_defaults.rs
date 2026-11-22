use betterlite_config::types::ModelDeployment;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_provider::anthropic_dispatch::base::{Surface, resolve_base};
use betterlite_provider::anthropic_dispatch::gateway::runtime::AnthropicBases;
use url::Url;

use crate::server::error::ChatSetupError;

// _complete_anthropic reads only the Anthropic chain: the OpenAI key and base resolve_with filled never apply
pub(super) fn apply_anthropic_defaults(
    resolved: &mut ChatDeployment,
    deployment: &ModelDeployment,
    bases: &AnthropicBases,
    surface: Surface,
) -> Result<(), ChatSetupError> {
    let base = resolve_base(
        surface,
        deployment.litellm_params.api_base.as_deref(),
        &bases.globals,
        &bases.environment,
        bases.disable_suffix,
    );
    resolved.endpoint = Url::parse(&base.url).map_err(|source| ChatSetupError::AnthropicBase {
        model_name: resolved.model_group.clone(),
        source,
    })?;
    resolved.api_base = base.url;
    resolved
        .api_key
        .clone_from(&deployment.litellm_params.api_key);
    resolved.organization = None;
    Ok(())
}
