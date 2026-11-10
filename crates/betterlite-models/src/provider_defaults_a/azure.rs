use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_a::environment::EnvironmentLookup;
use crate::provider_defaults_a::error::ProviderDefaultsError;
use crate::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest, ResolvedProvider,
};
use crate::provider_defaults_a::policies::ModelSets;
use crate::provider_defaults_a::table::{SimpleDefaults, simple_defaults};
use crate::provider_defaults_a::url;

pub(crate) const AZURE_AI: SimpleDefaults = SimpleDefaults {
    base_env: &["AZURE_AI_API_BASE"],
    base_default: None,
    key_env: &["AZURE_AI_API_KEY"],
    key_default: None,
};

// LiteLLM's constant; the branch treats this model as OpenAI whatever the catalog says.
const OPENAI_EMBEDDING_MODEL: &str = "text-embedding-ada-002";

pub(crate) fn azure_ai<E>(
    provider: ProviderId,
    request: ProviderDefaultsRequest,
    models: &ModelSets,
    env: &E,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    let api_key = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let (base, key) = simple_defaults(&AZURE_AI, request.api_base.as_deref(), api_key, env);
    let target = if is_azure_openai_model(&request.model, base.as_deref(), models)? {
        ProviderId::Azure
    } else {
        provider
    };
    Ok(ResolvedProvider {
        provider: DefaultsProvider::Known(target),
        model: request.model,
        api_base: base,
        api_key: key.map(SecretString::from),
    })
}

fn is_azure_openai_model(
    model: &str,
    api_base: Option<&str>,
    models: &ModelSets,
) -> Result<bool, ProviderDefaultsError> {
    let Some(base) = api_base else {
        return Ok(false);
    };
    let parts = url::parse(base).map_err(|url_error| ProviderDefaultsError::InvalidApiBase {
        api_base: base.to_owned(),
        source: url_error,
    })?;
    if is_foundry_inference_base(&parts) {
        return Ok(false);
    }
    let stripped = model.split_once('/').map_or(model, |(_, rest)| rest);
    Ok(models.openai_chat.contains(stripped)
        || models.openai_text_completion.contains(stripped)
        || stripped == OPENAI_EMBEDDING_MODEL)
}

// AzureAIChatConfig._is_foundry_model_inference_base: a Foundry inference host without a deployments path.
fn is_foundry_inference_base(parts: &url::UrlParts) -> bool {
    parts
        .hostname()
        .is_some_and(|host| host.ends_with(".services.ai.azure.com"))
        && !parts.path.contains("/openai/deployments")
}
