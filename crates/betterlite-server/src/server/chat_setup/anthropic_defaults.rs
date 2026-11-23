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
    let configured = deployment.litellm_params.api_base.as_deref();
    let base = resolve_base(
        surface,
        configured,
        &bases.globals,
        &bases.environment,
        bases.disable_suffix,
    );
    resolved.endpoint = Url::parse(&base.url).map_err(|source| ChatSetupError::AnthropicBase {
        model_name: resolved.model_group.clone(),
        source,
    })?;
    // get_api_base reports the deployment's own api_base; the adapter appends the suffix when it calls
    configured
        .unwrap_or_default()
        .clone_into(&mut resolved.api_base);
    resolved
        .api_key
        .clone_from(&deployment.litellm_params.api_key);
    resolved.organization = None;
    Ok(())
}
