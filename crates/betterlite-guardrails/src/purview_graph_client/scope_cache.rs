use std::sync::PoisonError;
use std::time::{Duration, SystemTime};

use betterlite_http::url_path::encode_segment;
use betterlite_prompts::arguments::json::Object;
use reqwest::header::{ETAG, HeaderMap};

use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::graph_api::graph_post;
use crate::purview_graph_client::wire::{PolicyLocation, ProtectionScopesRequest};
use crate::purview_graph_client::{PurviewGuardrailBase, SCOPE_CACHE_TTL};

#[derive(Debug)]
pub struct CachedScope {
    pub etag: String,
    pub response: Object,
    pub fetched_at: SystemTime,
}

fn cached_scopes(
    base: &PurviewGuardrailBase,
    user_id: &str,
    now: SystemTime,
) -> Option<(String, Object)> {
    base.scope_and_token_cache_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .protection_scope_cache
        .get(user_id)
        .filter(|entry| {
            now.duration_since(entry.fetched_at)
                .unwrap_or(Duration::ZERO)
                < SCOPE_CACHE_TTL
        })
        .map(|entry| (entry.etag.clone(), entry.response.clone()))
}

pub async fn compute_protection_scopes(
    base: &PurviewGuardrailBase,
    user_id: &str,
) -> Result<(String, Object), PurviewGraphError> {
    let encoded_user_id = encode_segment(user_id, "user_id")
        .map_err(|source| PurviewGraphError::UserId { source })?;
    if let Some(cached) = cached_scopes(base, user_id, (base.clock)()) {
        return Ok(cached);
    }

    let url = format!(
        "{}/users/{encoded_user_id}/dataSecurityAndGovernance/protectionScopes/compute",
        base.graph_api_base
    );
    let body = ProtectionScopesRequest {
        activities: "uploadText,downloadText",
        locations: [PolicyLocation {
            odata_type: "microsoft.graph.policyLocationApplication",
            value: &base.client_id,
        }],
    };
    let (response_json, response_headers) =
        graph_post(base, &url, &body, &HeaderMap::new()).await?;
    let etag = response_headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    // The TTL counts from when the scope response arrived, not from when the request started.
    let fetched_at = (base.clock)();
    base.scope_and_token_cache_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .protection_scope_cache
        .put(
            user_id.to_owned(),
            CachedScope {
                etag: etag.clone(),
                response: response_json.clone(),
                fetched_at,
            },
        );
    Ok((etag, response_json))
}

mod tests;
