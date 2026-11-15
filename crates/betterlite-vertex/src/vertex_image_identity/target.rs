use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::headers::set_headers;
use crate::model_urls::base_url::get_vertex_base_url;
use crate::token_cache::CredentialCache;

use super::error::ImageIdentityError;
use super::settings::ImageIdentitySettings;

pub struct ImageTarget<'request> {
    pub model: &'request str,
    pub api_base: Option<&'request str>,
    pub params: VertexParams<'request>,
}

pub struct ValidationRequest<'request> {
    pub params: VertexParams<'request>,
    // the deployment's api_base and the one inside litellm_params, which LiteLLM checks separately
    pub api_base: Option<&'request str>,
    pub params_api_base: Option<&'request str>,
    pub headers: &'request BTreeMap<String, String>,
}

// get_complete_url: a truthy custom base is the whole URL, and only then are project and location required
pub fn complete_url(
    settings: &ImageIdentitySettings,
    target: &ImageTarget<'_>,
) -> Result<String, ImageIdentityError> {
    let model_name = if target.model.starts_with("vertex_ai/") {
        target.model.replace("vertex_ai/", "")
    } else {
        target.model.to_owned()
    };
    if let Some(api_base) = target.api_base.filter(|base| !base.is_empty()) {
        return Ok(api_base.trim_end_matches('/').to_owned());
    }
    let project = settings
        .resolve_project(target.params)
        .ok_or(ImageIdentityError::MissingTarget)?;
    let location = settings
        .resolve_location(target.params)
        .ok_or(ImageIdentityError::MissingTarget)?;
    let base_url = get_vertex_base_url(Some(location)).map_err(ImageIdentityError::Location)?;
    Ok(format!(
        "{base_url}/v1/projects/{project}/locations/{location}/publishers/google/models/{model_name}:generateContent"
    ))
}

// any non-None api_base skips auth, but only a non-empty one becomes the URL
pub async fn validate_environment(
    cache: &CredentialCache,
    settings: &ImageIdentitySettings,
    request: &ValidationRequest<'_>,
) -> Result<BTreeMap<String, SecretString>, ImageIdentityError> {
    let custom_base =
        request.params_api_base.is_some_and(|base| !base.is_empty()) || request.api_base.is_some();
    if custom_base {
        return Ok(request
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
            .collect());
    }
    let project = settings.resolve_project(request.params);
    let credentials = settings.resolve_credentials(request.params);
    let access = cache
        .get_access_token(credentials, project)
        .await
        .map_err(ImageIdentityError::Auth)?;
    Ok(set_headers(Some(&access.token), request.headers))
}
