pub mod error;
pub mod file_routing;
pub mod model_hint;
pub mod provider_permissions;
pub mod response_ids;
pub mod store_overlay;
pub mod team_fallback;

mod tests;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::route_match::canonical::RequestRouteSource;
use betterlite_core::api::file_routing_ids::ManagedResourceAccessChecker;
use betterlite_core::api::request_body::JsonObject as RequestBody;
use betterlite_core::api::response_synthetic_events::python_value::has_content;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use http::HeaderMap;
use serde::Deserialize;

// What a files route reads off the request besides the request data it already decoded.
pub struct RequestFacts<'request> {
    pub method: &'request str,
    pub query: Option<&'request str>,
    pub headers: &'request HeaderMap,
    pub body: &'request RequestBody,
    pub route: RequestRouteSource<'request>,
}

// The authenticated caller: the principal the grant checks run for, and the key whose team and metadata these helpers read.
pub struct Caller<'caller> {
    pub principal: &'caller Principal,
    pub auth: &'caller UserAPIKeyAuth,
}

// The router, grant and managed-file ports a routing helper calls; a route with no router passes None.
pub struct RoutingDeps<'deps, Catalog, Grants> {
    pub catalog: Option<&'deps Catalog>,
    pub grants: &'deps Grants,
    pub managed_files: Option<&'deps dyn ManagedResourceAccessChecker>,
    pub require_managed_files: bool,
}

// The store a route already resolved and the registry B1062 loaded, for the legacy overlay path.
pub struct StoreDeps<'store> {
    pub managed_store: Option<&'store LiteLlmManagedVectorStore>,
    pub registry: Option<&'store VectorStoreRegistry>,
    pub should_lookup_registry: bool,
}

// The key fields these helpers read, taken from the auth's serde form as B1020 and B1061 read their grants.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct KeyFacts {
    pub(super) team_id: Option<String>,
    pub(super) team_models: Option<Json>,
    pub(super) metadata: Option<JsonObject>,
    pub(super) team_metadata: Option<JsonObject>,
}

// A header as starlette decodes it (latin-1, first value), the reading B10059's provider getter uses.
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).map(|value| {
        value
            .as_bytes()
            .iter()
            .map(|byte| char::from(*byte))
            .collect()
    })
}

// starlette's query_params.get: the last value carrying the name, and None when the name is absent.
fn last_query_value(query: Option<&str>, name: &str) -> Option<String> {
    let mut found = None;
    for (key, value) in form_urlencoded::parse(query?.as_bytes()) {
        if key == name {
            found = Some(value.into_owned());
        }
    }
    found
}
