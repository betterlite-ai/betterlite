use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse as _, Response};
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
use endpoints::load_bundled_endpoints;
use error::ProvidersError;
use types::SupportedEndpointsResponse;

// Built once in providers_routes and shared through router state, since the bundled file never changes at runtime.
type EndpointsState = Arc<Result<SupportedEndpointsResponse, ProvidersError>>;

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

pub async fn get_provider_fields() -> Result<Response, ProvidersError> {
    match provider_catalog() {
        Ok(fields) => Ok(Json(fields).into_response()),
        Err(error) => Err(error),
    }
}

pub async fn get_supported_endpoints(
    State(endpoints): State<EndpointsState>,
) -> Result<Response, ProvidersError> {
    match &*endpoints {
        Ok(response) => Ok(Json(response.clone()).into_response()),
        Err(error) => Err(error.clone()),
    }
}

// Awaits a future B003 integration task that merges this into the server's main router.
pub fn providers_routes() -> Router {
    let endpoints: EndpointsState = Arc::new(load_bundled_endpoints());
    Router::new()
        .route("/public/providers", get(get_supported_providers))
        .route("/public/providers/fields", get(get_provider_fields))
        .route("/public/endpoints", get(get_supported_endpoints))
        .with_state(endpoints)
}
