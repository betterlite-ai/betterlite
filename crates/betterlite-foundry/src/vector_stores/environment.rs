use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::azure_urls::{EnvironmentHeadersRequest, validate_azure_environment};
use betterlite_azure::credentials::source::{CallableTokenProvider, CredentialSources};
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use secrecy::{ExposeSecret, SecretString};

use super::error::AzureSearchError;

pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, SecretString>;

const API_KEY_HEADER: &str = "api-key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const SERVICE_DOMAIN: &str = ".search.windows.net";

// the typed auth record holds header values as untyped JSON, so this leaf keeps the key in a secret instead
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCredentials {
    pub headers: Headers,
}

// LiteLLM matches these as literal substrings, with writes checked first so an index named analyze cannot shadow a batch write
#[must_use]
pub fn endpoints_by_type() -> VectorStoreIndexEndpoints {
    let read = [
        ("GET", "/indexes/"),
        ("POST", "/docs/search"),
        ("POST", "/docs/suggest"),
        ("POST", "/docs/autocomplete"),
        ("POST", "/analyze"),
    ]
    .iter()
    .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
    .collect();
    let write = [("POST", "/docs/index")]
        .iter()
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect();
    VectorStoreIndexEndpoints {
        read: Some(read),
        write: Some(write),
        extra: BTreeMap::new(),
    }
}

pub fn auth_credentials(litellm_params: &Params) -> Result<AuthCredentials, AzureSearchError> {
    let api_key = match litellm_params.get("api_key") {
        None | Some(Json::Null) => return Err(AzureSearchError::MissingApiKey),
        Some(Json::Text(key)) => key,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AzureSearchError::InvalidField {
                field: "api_key",
                expected: "a string",
            });
        }
    };
    let headers = Headers::from([(
        API_KEY_HEADER.to_owned(),
        SecretString::from(api_key.clone()),
    )]);
    Ok(AuthCredentials { headers })
}

pub struct EnvironmentRequest<'request> {
    pub headers: Headers,
    pub api_key: Option<&'request SecretString>,
    pub credentials: &'request DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'request>>,
    pub sources: &'request dyn CredentialSources,
}

pub fn validate_environment(request: EnvironmentRequest<'_>) -> Result<Headers, AzureSearchError> {
    let exposed = request
        .headers
        .into_iter()
        .map(|(name, value)| (name, value.expose_secret().to_owned()))
        .collect();
    let validated = validate_azure_environment(EnvironmentHeadersRequest {
        headers: exposed,
        api_key: request.api_key,
        credentials: request.credentials,
        refresh_enabled: request.refresh_enabled,
        provider: request.provider,
        sources: request.sources,
    })
    .map_err(|source| AzureSearchError::Token { source })?;
    let mut headers: Headers = validated
        .into_iter()
        .map(|(name, value)| (name, SecretString::from(value)))
        .collect();
    headers.insert(
        CONTENT_TYPE_HEADER.to_owned(),
        SecretString::from(JSON_CONTENT_TYPE),
    );
    Ok(headers)
}

pub fn complete_url(
    api_base: Option<&str>,
    litellm_params: &Params,
) -> Result<String, AzureSearchError> {
    if let Some(base) = api_base.filter(|value| !value.is_empty()) {
        return Ok(base.trim_end_matches('/').to_owned());
    }
    match litellm_params.get("azure_search_service_name") {
        Some(Json::Text(name)) if !name.is_empty() => Ok(format!("https://{name}{SERVICE_DOMAIN}")),
        None | Some(Json::Null | Json::Text(_)) => Err(AzureSearchError::MissingServiceName),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AzureSearchError::InvalidField {
                field: "azure_search_service_name",
                expected: "a string",
            })
        }
    }
}
