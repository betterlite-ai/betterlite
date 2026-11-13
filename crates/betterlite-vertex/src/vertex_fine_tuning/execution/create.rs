use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::catalog::ModelCatalog;
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use http::Method;
use url::Url;

use crate::cache_lookup::transport::cache_headers;
use crate::endpoint_target::target::{
    EndpointTarget, TargetProvider, TokenUrlInput, get_token_and_url,
};
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::model_urls::base_url::get_vertex_base_url;
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::{ApiVersion, UrlMode};
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::token_refresh::ensure_access_token;
use crate::vertex_fine_tuning::records::request::{OriginalHyperparameters, to_vertex_request};
use crate::vertex_fine_tuning::records::response::decode_vertex_tuning_job;

use super::error::TuningExecutionError;
use super::transport::TuningTransport;

// LiteLLM builds a sync client with timeout 600 and connect 5, and ignores the one the caller passed
pub const TUNING_TIMEOUT: Duration = Duration::from_secs(600);

const OK_STATUS: u16 = 200;

pub struct TuningCreateRequest<'request> {
    pub catalog: &'request ModelCatalog,
    pub data: &'request FineTuneCreate,
    pub original_hyperparameters: &'request OriginalHyperparameters,
    pub credentials: Option<&'request ConfiguredCredentials>,
    pub project: Option<&'request str>,
    pub location: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub model: &'request str,
    pub local: &'request LocalTimeContext,
    // LiteLLM ignores the caller's timeout for tuning jobs; the field keeps the call site uniform
    pub timeout: Option<Duration>,
}

pub async fn create_tuning_job<T, Make, Lookup>(
    transport: &T,
    request: &TuningCreateRequest<'_>,
    make_lookup: Make,
) -> Result<FineTuneJob, TuningExecutionError>
where
    T: TuningTransport,
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    let (access, prepared) = resolve_target(request, make_lookup).await?;
    let job = to_vertex_request(request.data, request.original_hyperparameters)?;
    let body =
        serde_json::to_vec(&job).map_err(|source| TuningExecutionError::Encoded { source })?;
    post_tuning_job(transport, request, &prepared, &access.project, body).await
}

async fn resolve_target<Make, Lookup>(
    request: &TuningCreateRequest<'_>,
    make_lookup: Make,
) -> Result<(AccessToken, EndpointTarget), TuningExecutionError>
where
    Make: FnOnce(Option<&ConfiguredCredentials>, Option<&str>) -> Lookup,
    Lookup: Future<Output = Result<AccessToken, CredentialCacheError>> + Send,
{
    // B1302 resolves the credentials before anything is prepared or sent
    let access = ensure_access_token(
        VertexCapabilityProvider::VertexAiBeta,
        make_lookup(request.credentials, request.project),
    )
    .await
    .map_err(|source| TuningExecutionError::Credentials { source })?;
    // B1304 prepares the token and URL; only its auth header is used, since the POST target is the tuningJobs URL
    let prepared = get_token_and_url(
        request.catalog,
        &TokenUrlInput {
            provider: TargetProvider::Vertex,
            model: request.model,
            auth_token: Some(&access.token),
            gemini_api_key: None,
            project: Some(access.project.as_str()),
            location: request.location,
            stream: false,
            api_base: request.api_base,
            api_version: ApiVersion::V1,
            mode: UrlMode::Chat,
            use_psc_endpoint_format: false,
        },
    )
    .map_err(|source| TuningExecutionError::Target { source })?;
    Ok((access, prepared))
}

async fn post_tuning_job<T>(
    transport: &T,
    request: &TuningCreateRequest<'_>,
    prepared: &EndpointTarget,
    project: &str,
    body: Vec<u8>,
) -> Result<FineTuneJob, TuningExecutionError>
where
    T: TuningTransport,
{
    let headers = cache_headers(prepared, &BTreeMap::new())
        .map_err(|source| TuningExecutionError::Header { source })?;
    let outbound = OutboundRequest {
        method: Method::POST,
        url: tuning_url(request.location, project)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline: TUNING_TIMEOUT,
    };
    let reply = transport
        .send(outbound)
        .await
        .map_err(|source| TuningExecutionError::Transport { source })?;
    if reply.status != OK_STATUS {
        return Err(TuningExecutionError::Http {
            status: reply.status,
            body: str::from_utf8(&reply.body).map_or_else(
                |_error| format!("{} bytes that are not UTF-8", reply.body.len()),
                str::to_owned,
            ),
        });
    }
    decode_vertex_tuning_job(&reply.body, request.local).map_err(TuningExecutionError::Record)
}

fn tuning_url(location: Option<&str>, project: &str) -> Result<Url, TuningExecutionError> {
    let Some(location) = location else {
        return Err(TuningExecutionError::VertexUrl(
            VertexUrlError::LocationRequired,
        ));
    };
    let base = get_vertex_base_url(Some(location))?;
    let text = format!("{base}/v1/projects/{project}/locations/{location}/tuningJobs");
    Url::parse(&text).map_err(|source| TuningExecutionError::Url { source })
}
