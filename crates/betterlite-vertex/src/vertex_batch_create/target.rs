use url::Url;

use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::graft::{graft_default_vertex_path, parse_base};
use crate::endpoint_target::target::{CustomProxyInput, TargetProvider, check_custom_proxy};
use crate::model_urls::ApiVersion;
use crate::model_urls::base_url::get_vertex_base_url;

use super::error::VertexBatchCreateError;

mod tests;

pub struct BatchTargetInput<'text> {
    pub api_base: Option<&'text str>,
    pub project: &'text str,
    pub location: &'text str,
    // the resource name a tuned endpoint is read from, projects/<project>/locations/<location>/endpoints/<id>
    pub model: &'text str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchTarget {
    pub collection: String,
    pub endpoint: String,
}

// the two URLs the create call can reach, built without any lookup of the tuned model
pub fn vertex_batch_target(
    input: &BatchTargetInput<'_>,
) -> Result<BatchTarget, VertexBatchCreateError> {
    Ok(BatchTarget {
        collection: collection_url(input.api_base, input.project, input.location)?,
        endpoint: endpoint_resolution_url(input.api_base, input.model, input.location)?,
    })
}

pub fn collection_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
) -> Result<String, VertexBatchCreateError> {
    let host = get_vertex_base_url(Some(location)).map_err(EndpointTargetError::VertexUrl)?;
    let default_url =
        format!("{host}/v1/projects/{project}/locations/{location}/batchPredictionJobs");
    // LiteLLM splits the whole URL on ':' and passes the last piece as the endpoint, so the scheme colon leaves "//host/v1/..." behind
    let endpoint = default_url
        .rsplit_once(':')
        .map_or("", |(_head, tail)| tail);
    let target = check_custom_proxy(&CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint,
        stream: false,
        auth_header: None,
        url: &default_url,
        model: None,
        project: Some(project),
        location: Some(location),
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

// a custom base replaces the Google host: /v1 and /v1beta1 swallow the version segment, any other path stays as a mount prefix, and no ":operation" suffix applies to a plain GET
pub fn endpoint_resolution_url(
    api_base: Option<&str>,
    model: &str,
    location: &str,
) -> Result<String, VertexBatchCreateError> {
    let host = get_vertex_base_url(Some(location)).map_err(EndpointTargetError::VertexUrl)?;
    let default_url = format!("{host}/v1/{model}");
    let Some(base) = api_base.filter(|base| !base.is_empty()) else {
        return Ok(default_url);
    };
    let parsed_base = parse_base(base)?;
    if matches!(parsed_base.path().trim_end_matches('/'), "/v1" | "/v1beta1") {
        return Ok(graft_default_vertex_path(base, &default_url)?);
    }
    let default_path = Url::parse(&default_url)
        .map_err(|source| VertexBatchCreateError::Url { source })?
        .path()
        .to_owned();
    Ok(format!("{}{default_path}", base.trim_end_matches('/')))
}
