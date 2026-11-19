use std::future::Future;

use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::beta_headers::ANTHROPIC_BETA_HEADER;
use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret as _;

use super::beta::beta_tokens;
use super::error::VertexMessagesError;
use crate::credential_sources::getters::VertexParams;
use crate::endpoint_target::partner::Partner;
use crate::endpoint_target::target::{PartnerRequest, get_complete_vertex_url};
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;

pub const AUTHORIZATION: &str = "Authorization";
pub const CONTENT_TYPE: &str = "content-type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
const STREAM: &str = "stream";

pub struct EnvironmentRequest<'request> {
    pub headers: &'request AnthropicHeaders,
    pub model: &'request str,
    pub messages: &'request [Json],
    pub optional_params: &'request Object,
    pub params: VertexParams<'request>,
    pub settings: &'request VertexSettings,
    pub catalog: &'request ModelCatalog,
    pub api_base: Option<&'request str>,
}

#[derive(Debug)]
pub struct Environment {
    pub headers: AnthropicHeaders,
    pub api_base: String,
}

// LiteLLM's validate_anthropic_messages_environment: a copied header map, a fresh Bearer token, the Claude partner URL and the beta set.
pub async fn validate_environment<Make, Lookup>(
    request: &EnvironmentRequest<'_>,
    make_lookup: Make,
) -> Result<Environment, VertexMessagesError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup + Send,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let mut headers = request.headers.clone();
    let access = resolve_access(request, make_lookup).await?;
    headers.values.insert(
        AUTHORIZATION.to_owned(),
        format!("Bearer {}", access.token.expose_secret()),
    );
    let api_base = partner_url(request, &access)?;
    headers
        .values
        .insert(CONTENT_TYPE.to_owned(), JSON_CONTENT_TYPE.to_owned());
    add_betas(&mut headers, request)?;
    Ok(Environment { headers, api_base })
}

// LiteLLM's _ensure_access_token: the caller's project wins inside get_access_token, so the lookup only sees the configured one.
async fn resolve_access<Make, Lookup>(
    request: &EnvironmentRequest<'_>,
    make_lookup: Make,
) -> Result<AccessToken, VertexMessagesError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup + Send,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let project = request.params.safe_get_vertex_ai_project(request.settings);
    let credentials = request
        .params
        .safe_get_vertex_ai_credentials(request.settings);
    ensure_access_token(
        VertexCapabilityProvider::VertexAi,
        make_lookup(credentials, project),
    )
    .await
    .map_err(|source| VertexMessagesError::Credentials { source })
}

fn partner_url(
    request: &EnvironmentRequest<'_>,
    access: &AccessToken,
) -> Result<String, VertexMessagesError> {
    get_complete_vertex_url(
        request.catalog,
        &PartnerRequest {
            custom_api_base: request.api_base,
            location: request.params.safe_get_vertex_ai_location(request.settings),
            vertex_project: request.params.safe_get_vertex_ai_project(request.settings),
            project_id: &access.project,
            partner: Partner::Claude,
            stream: matches!(request.optional_params.get(STREAM), Some(Json::Bool(true))),
            model: request.model,
        },
    )
    .map_err(VertexMessagesError::Endpoint)
}

fn add_betas(
    headers: &mut AnthropicHeaders,
    request: &EnvironmentRequest<'_>,
) -> Result<(), VertexMessagesError> {
    let betas = beta_tokens(headers, request.messages, request.optional_params)?;
    if betas.is_empty() {
        return Ok(());
    }
    // LiteLLM joins a set, so its order is unspecified; the sorted tokens keep the header run-stable.
    let joined = betas
        .iter()
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .join(",");
    headers
        .values
        .insert(ANTHROPIC_BETA_HEADER.to_owned(), joined);
    Ok(())
}

// LiteLLM's Vertex get_complete_url: validation already built the endpoint, so a missing base is the only failure.
pub fn get_complete_url(api_base: Option<&str>) -> Result<String, VertexMessagesError> {
    api_base
        .map(str::to_owned)
        .ok_or(VertexMessagesError::MissingApiBase)
}
