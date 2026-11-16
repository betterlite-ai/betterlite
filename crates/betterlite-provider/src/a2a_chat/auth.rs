use std::collections::BTreeMap;

use betterlite_foundry::credentials::agent::has_azure_entra_params;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

// LiteLLM's AgentAuthHeaderResolver: the Entra resolver behind the hop header and the agent key
pub trait EntraResolver: Send + Sync {
    fn token(&self, params: &Object) -> Result<SecretString, CredentialsError>;
    fn auth_header(&self, params: &Object) -> Result<BTreeMap<String, String>, CredentialsError>;
}

#[must_use]
pub fn a2a_hop_uses_entra(params: &Object, custom_llm_provider: Option<&str>) -> bool {
    if !custom_llm_provider.is_none_or(str::is_empty) {
        return false;
    }
    has_azure_entra_params(Some(params))
}

pub fn resolve_a2a_hop_auth_header(
    params: &Object,
    custom_llm_provider: Option<&str>,
    resolver: &dyn EntraResolver,
) -> Result<Option<BTreeMap<String, String>>, CredentialsError> {
    if !a2a_hop_uses_entra(params, custom_llm_provider) {
        return Ok(None);
    }
    resolver.auth_header(params).map(Some)
}
