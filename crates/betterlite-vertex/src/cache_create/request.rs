use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::Method;
use serde::Serialize;
use url::Url;

use crate::cache_lookup::error::CacheLookupError;
use crate::cache_lookup::transport::cache_headers;
use crate::cache_prefix::transform::CachedContentRequestBody;
use crate::endpoint_target::target::EndpointTarget;

use super::error::CacheCreateError;

// LiteLLM always writes tools, null when the request carried none, and toolConfig only when a choice was sent
#[derive(Debug, Serialize)]
pub struct CreateCacheBody<'request> {
    #[serde(flatten)]
    pub cached: CachedContentRequestBody,
    pub tools: Option<&'request [Json]>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<&'request Json>,
}

pub fn cache_create_request(
    target: &EndpointTarget,
    extra_headers: &BTreeMap<String, String>,
    body: Vec<u8>,
    deadline: Duration,
) -> Result<OutboundRequest, CacheCreateError> {
    let url = Url::parse(&target.url).map_err(|source| CacheCreateError::Endpoint {
        source: CacheLookupError::Url { source },
    })?;
    let headers = cache_headers(target, extra_headers)
        .map_err(|source| CacheCreateError::Endpoint { source })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline,
    })
}
