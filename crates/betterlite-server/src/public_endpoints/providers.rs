use axum::routing::get;
use axum::{Json, Router};
use betterlite_models::identity::provider_id::ProviderId;
use strum::VariantArray as _;

pub mod catalog;
mod endpoint_metadata;
pub mod endpoints;
pub mod error;
mod tests;
pub mod types;

use catalog::provider_catalog;
use endpoints::supported_endpoints;
use types::{ProviderCreateInfo, SupportedEndpointsResponse};

// JSONProviderRegistry-only slugs (providers.json, B1424): no member in litellm.types.utils.LlmProviders.
const EXCLUDED_FROM_LLM_PROVIDERS: [&str; 8] = [
    "abliteration",
    "llamagate",
    "gmi",
    "sarvam",
    "aihubmix",
    "crusoe",
    "empiriolabs",
    "veniceai",
];

pub async fn get_supported_providers() -> Json<Vec<String>> {
    let mut providers: Vec<String> = ProviderId::VARIANTS
        .iter()
        .map(|provider| <&str>::from(*provider))
        .filter(|slug| !EXCLUDED_FROM_LLM_PROVIDERS.contains(slug))
        .map(str::to_owned)
        .collect();
    providers.sort();
    Json(providers)
}

pub async fn get_provider_fields() -> Json<Vec<ProviderCreateInfo>> {
    provider_catalog().map_or_else(
        |error| {
            tracing::warn!(%error, "bundled provider catalog failed to decode");
            Json(Vec::new())
        },
        Json,
    )
}

pub async fn get_supported_endpoints() -> Json<SupportedEndpointsResponse> {
    supported_endpoints().map_or_else(
        |_| Json(SupportedEndpointsResponse::default()),
        |cached| Json((*cached).clone()),
    )
}

// Awaits a future B003 integration task that merges this into the server's main router.
pub fn providers_routes() -> Router {
    Router::new()
        .route("/public/providers", get(get_supported_providers))
        .route("/public/providers/fields", get(get_provider_fields))
        .route("/public/endpoints", get(get_supported_endpoints))
}
