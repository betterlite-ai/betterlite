use std::collections::BTreeMap;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::credentials::AZURE_AI_API_KEY_VAR;
use crate::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use crate::credentials::host::ApiKeyHeader;
use crate::credentials::model_info::ModelInfoAccessors;

use super::error::ImageEditError;
use super::selection::ImageEditVariant;

const CONTENT_TYPE: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

pub struct ValidateEnvironmentRequest<'data> {
    pub headers: Headers,
    pub model: &'data str,
    pub api_key: Option<&'data SecretString>,
    pub params: Option<&'data Object>,
    pub environment: &'data BTreeMap<String, String>,
    pub accessors: ModelInfoAccessors<'data>,
    pub refresh_enabled: bool,
    pub sources: &'data dyn CredentialSources,
}

// All three configs resolve the key through B549 and fall back to Entra; only FLUX 2 overwrites Content-Type with JSON
pub fn validate_environment(
    request: ValidateEnvironmentRequest<'_>,
) -> Result<Headers, ImageEditError> {
    let variant = ImageEditVariant::select(request.model)?;
    let resolved = request.accessors.api_key(request.api_key);
    let auth = get_azure_ai_auth_headers(&AuthHeaderRequest {
        api_key: resolved.as_ref(),
        params: request.params,
        environment: request.environment,
        api_key_header: key_header(variant),
        api_key_env_var: AZURE_AI_API_KEY_VAR,
        refresh_enabled: request.refresh_enabled,
        sources: request.sources,
    })?;
    let mut headers = request.headers;
    headers.extend(auth);
    if variant == ImageEditVariant::Flux2 {
        headers.insert(String::from(CONTENT_TYPE), String::from(JSON_CONTENT_TYPE));
    }
    Ok(headers)
}

const fn key_header(variant: ImageEditVariant) -> ApiKeyHeader {
    match variant {
        ImageEditVariant::Flux1 | ImageEditVariant::Flux2 => ApiKeyHeader::ApiKeyTitleCase,
        ImageEditVariant::Mai => ApiKeyHeader::ApiKey,
    }
}
