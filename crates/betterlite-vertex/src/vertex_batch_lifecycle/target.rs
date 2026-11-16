use crate::endpoint_target::target::{CustomProxyInput, TargetProvider, check_custom_proxy};
use crate::model_urls::ApiVersion;

use super::error::VertexBatchLifecycleError;

mod tests;

pub(super) fn proxy_url(
    api_base: Option<&str>,
    default: &str,
    endpoint: &str,
    proxy_project: Option<&str>,
    proxy_location: Option<&str>,
) -> Result<String, VertexBatchLifecycleError> {
    let target = check_custom_proxy(&CustomProxyInput {
        api_base,
        provider: TargetProvider::Vertex,
        gemini_api_key: None,
        endpoint,
        stream: false,
        auth_header: None,
        url: default,
        model: None,
        project: proxy_project,
        location: proxy_location,
        api_version: ApiVersion::V1,
        use_psc_endpoint_format: false,
    })?;
    Ok(target.url)
}

// LiteLLM reads the endpoint from the last colon of the URL it built, and the scheme colon is part of that reading
pub(super) fn endpoint_tail(default: &str) -> &str {
    match default.rsplit_once(':') {
        Some((_head, tail)) => tail,
        None => "",
    }
}
