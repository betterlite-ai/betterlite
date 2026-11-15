use std::collections::BTreeMap;

use betterlite_azure::credentials::selection::{AzureCredentialRequest, get_azure_ad_token};
use betterlite_azure::credentials::source::CredentialSources;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use super::error::CredentialsError;
use super::params::EntraParams;

pub struct EntraTokenRequest<'data> {
    pub params: Option<&'data Object>,
    pub environment: &'data BTreeMap<String, String>,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

pub fn get_azure_ai_entra_token(
    request: &EntraTokenRequest<'_>,
) -> Result<Option<SecretString>, CredentialsError> {
    let params = EntraParams::from_mapping(request.params)?;
    let settings = params.deployment_settings(request.environment);
    let credential = AzureCredentialRequest {
        params: &settings,
        refresh_enabled: request.refresh_enabled,
        provider: None,
        sources: request.sources,
    };
    Ok(get_azure_ad_token(&credential)?)
}
