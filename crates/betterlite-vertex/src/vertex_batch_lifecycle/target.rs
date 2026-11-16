use betterlite_http::url_path::encode_segment;

use crate::endpoint_target::target::{CustomProxyInput, TargetProvider, check_custom_proxy};
use crate::model_urls::ApiVersion;
use crate::vertex_batch_create::target::collection_url;

use super::error::VertexBatchLifecycleError;

mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelTarget {
    pub cancel: String,
    pub retrieve: String,
}

pub fn retrieve_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    batch_id: &str,
) -> Result<String, VertexBatchLifecycleError> {
    let encoded = encode_segment(batch_id, "batch_id")?;
    let default = format!("{}/{}", collection_url(None, project, location)?, encoded);
    let endpoint = default.rsplit_once(':').map_or("", |(_head, tail)| tail);
    let target = check_custom_proxy(&CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint,
        stream: false,
        auth_header: None,
        url: &default,
        model: None,
        project: Some(project),
        location: Some(location),
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

// LiteLLM's list call omits the project, location and version arguments, and its non-PSC path reads none of them
pub fn list_url(
    api_base: Option<&str>,
    project: &str,
    location: &str,
) -> Result<String, VertexBatchLifecycleError> {
    let default = collection_url(None, project, location)?;
    let endpoint = default.rsplit_once(':').map_or("", |(_head, tail)| tail);
    let target = check_custom_proxy(&CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint,
        stream: false,
        auth_header: None,
        url: &default,
        model: None,
        project: None,
        location: None,
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

pub fn cancel_target(
    api_base: Option<&str>,
    project: &str,
    location: &str,
    batch_id: &str,
) -> Result<CancelTarget, VertexBatchLifecycleError> {
    let encoded = encode_segment(batch_id, "batch_id")?;
    let default = format!(
        "{}/{}:cancel",
        collection_url(None, project, location)?,
        encoded
    );
    let target = check_custom_proxy(&CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint: "cancel",
        stream: false,
        auth_header: None,
        url: &default,
        model: None,
        project: Some(project),
        location: Some(location),
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    let retrieve = retrieval_url(&target.url);
    Ok(CancelTarget {
        cancel: target.url,
        retrieve,
    })
}

// LiteLLM removes one terminal ":cancel" as text, and only the fallback path trims slashes off the remainder
fn retrieval_url(cancel_url: &str) -> String {
    match cancel_url.strip_suffix(":cancel") {
        Some(retrieve) => retrieve.to_owned(),
        None => cancel_url
            .rsplit_once(":cancel")
            .map_or(cancel_url, |(head, _tail)| head)
            .trim_end_matches('/')
            .to_owned(),
    }
}
