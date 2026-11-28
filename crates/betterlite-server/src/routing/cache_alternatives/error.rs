use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_rate_limit::pre_call_hook::PreCallHookError;
use betterlite_router::deployment_catalog::error::CatalogLookupError;
use betterlite_router::router_core::error::RoutingError;
use thiserror::Error;

// Every way an optional cache-routing step can stop; choose_cached_model turns each into the classified route.
#[derive(Debug, Error)]
pub enum CacheAlternativesError {
    // sources are boxed to keep the enum under result_large_err's 128-byte limit
    #[error("the deployments for {model} cannot be listed: {source}")]
    Deployments {
        model: String,
        #[source]
        source: Box<CatalogLookupError>,
    },
    #[error("the healthy deployments for {model} cannot be read: {source}")]
    Healthy {
        model: String,
        #[source]
        source: Box<RoutingError>,
    },
    #[error("the caller's access to {model} cannot be checked: {source}")]
    Access {
        model: String,
        #[source]
        source: Box<EntityModelAccessError>,
    },
    #[error("request capacity for {model} was denied: {source}")]
    Capacity {
        model: String,
        #[source]
        source: Box<PreCallHookError>,
    },
    #[error(
        "request capacity cannot be taken for {model} because no capacity limiter is configured"
    )]
    CapacityUnavailable { model: String },
    #[error("the request metadata under {field} is not a mapping")]
    Metadata { field: &'static str },
    #[error("the caller's per-key config is not a mapping")]
    CallerConfig,
}
