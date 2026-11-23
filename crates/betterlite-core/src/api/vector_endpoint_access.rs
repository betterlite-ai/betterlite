pub mod error;

mod grants;

mod tests;

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::route_match::canonical::{RequestRouteSource, get_request_route};
use betterlite_auth::vector_index_admin::lifecycle::is_index_lifecycle_request;
use betterlite_auth::vector_index_admin::{
    IndexOperation, auth_is_proxy_admin, require_index_admin,
};
use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, VectorStoresConfigTag, select_store_files_config,
    select_vector_stores_config,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use serde::Deserialize;

use self::error::VectorEndpointAccessError;
use self::grants::{Grants, caller_grants};

// B010 selects the provider's vector store config; B527/B558 answer that config's endpoint table
pub trait VectorEndpointTables: Send + Sync {
    fn store_endpoints(&self, config: VectorStoresConfigTag) -> VectorStoreIndexEndpoints;

    fn file_endpoints(&self, config: VectorStoreFilesConfigTag) -> VectorStoreIndexEndpoints;
}

pub struct VectorEndpointAccess<'access> {
    pub provider: ProviderId,
    pub vector_store_id: &'access str,
    pub method: &'access str,
    pub route: RequestRouteSource<'access>,
    pub auth: &'access UserAPIKeyAuth,
    pub tables: &'access dyn VectorEndpointTables,
}

// what LiteLLM's Literal[True] | None says: the request passed, or this policy does not cover it
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointDecision {
    Allowed,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VectorStorePermission {
    Read,
    Write,
}

impl VectorStorePermission {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

// LiteLLM's _does_endpoint_match: a literal anywhere in the route, then only the prefix before the first brace
#[must_use]
pub fn does_endpoint_match(endpoint_path: &str, request_path: &str) -> bool {
    if request_path.contains(endpoint_path) {
        return true;
    }
    endpoint_path
        .split_once('{')
        .is_some_and(|(prefix, _rest)| !prefix.is_empty() && request_path.contains(prefix))
}

// LiteLLM's check_vector_store_permission: an exact index name and an exact permission, from either map
#[must_use]
pub fn check_vector_store_permission(
    index_name: &str,
    permission: VectorStorePermission,
    key_metadata: Option<&Payload>,
    team_metadata: Option<&Payload>,
) -> bool {
    [key_metadata, team_metadata]
        .into_iter()
        .flatten()
        .any(|metadata| metadata_allows(metadata, index_name, permission))
}

fn metadata_allows(
    metadata: &Payload,
    index_name: &str,
    permission: VectorStorePermission,
) -> bool {
    let Some(JsonData::Array(indexes)) = metadata.0.get("allowed_vector_store_indexes") else {
        return false;
    };
    for index_config in indexes {
        let JsonData::Object(entry) = index_config else {
            continue;
        };
        if entry_allows(entry, index_name, permission) {
            return true;
        }
    }
    false
}

fn entry_allows(
    entry: &BTreeMap<String, JsonData>,
    index_name: &str,
    permission: VectorStorePermission,
) -> bool {
    let Some(JsonData::String(name)) = entry.get("index_name") else {
        return false;
    };
    if name != index_name {
        return false;
    }
    let Some(JsonData::Array(permissions)) = entry.get("index_permissions") else {
        return false;
    };
    permissions.iter().any(|held| match held {
        JsonData::String(text) => text == permission.as_str(),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => false,
    })
}

// LiteLLM's is_allowed_to_call_vector_store_endpoint
pub fn store_endpoint_decision(
    access: &VectorEndpointAccess<'_>,
) -> Result<EndpointDecision, VectorEndpointAccessError> {
    if auth_is_proxy_admin(access.auth) {
        return Ok(EndpointDecision::Allowed);
    }
    let grants = caller_grants(access.auth)?;
    let Some(config) = select_vector_stores_config(access.provider, None) else {
        return Ok(EndpointDecision::NotApplicable);
    };
    let endpoints = access.tables.store_endpoints(config);
    let route = get_request_route(&access.route);
    if is_index_lifecycle_request(access.method, &route, access.vector_store_id) {
        let operation = match access.method {
            "DELETE" => IndexOperation::Delete,
            "PUT" | "PATCH" => IndexOperation::Update,
            _ => IndexOperation::Create,
        };
        require_index_admin(access.auth, operation)?;
        return Ok(EndpointDecision::Allowed);
    }
    let Some(permission) = classify_endpoint(&endpoints, access.method, &route) else {
        return Err(store_denied(access.vector_store_id));
    };
    if granted(access, &grants, permission) {
        return Ok(EndpointDecision::Allowed);
    }
    Err(store_denied(access.vector_store_id))
}

// LiteLLM's is_allowed_to_call_vector_store_files_endpoint: an unmatched endpoint is not this policy's
pub fn file_endpoint_decision(
    access: &VectorEndpointAccess<'_>,
) -> Result<EndpointDecision, VectorEndpointAccessError> {
    if auth_is_proxy_admin(access.auth) {
        return Ok(EndpointDecision::Allowed);
    }
    let grants = caller_grants(access.auth)?;
    let Some(config) = select_store_files_config(access.provider) else {
        return Ok(EndpointDecision::NotApplicable);
    };
    let endpoints = access.tables.file_endpoints(config);
    let route = get_request_route(&access.route);
    let Some(permission) = classify_endpoint(&endpoints, access.method, &route) else {
        return Ok(EndpointDecision::NotApplicable);
    };
    if granted(access, &grants, permission) {
        return Ok(EndpointDecision::Allowed);
    }
    Err(file_denied(access.vector_store_id))
}

// writes are classified first, so a route matching both patterns needs the stronger grant
fn classify_endpoint(
    endpoints: &VectorStoreIndexEndpoints,
    method: &str,
    route: &str,
) -> Option<VectorStorePermission> {
    if matches_endpoint(endpoints.write.as_deref(), method, route) {
        return Some(VectorStorePermission::Write);
    }
    matches_endpoint(endpoints.read.as_deref(), method, route)
        .then_some(VectorStorePermission::Read)
}

fn matches_endpoint(endpoints: Option<&[(String, String)]>, method: &str, route: &str) -> bool {
    endpoints
        .unwrap_or_default()
        .iter()
        .any(|(endpoint_method, endpoint_path)| {
            endpoint_method == method && does_endpoint_match(endpoint_path, route)
        })
}

fn granted(
    access: &VectorEndpointAccess<'_>,
    grants: &Grants,
    permission: VectorStorePermission,
) -> bool {
    check_vector_store_permission(
        access.vector_store_id,
        permission,
        grants.metadata.as_ref(),
        grants.team_metadata.as_ref(),
    )
}

fn store_denied(index_name: &str) -> VectorEndpointAccessError {
    VectorEndpointAccessError::StoreDenied {
        index_name: index_name.to_owned(),
    }
}

fn file_denied(vector_store_id: &str) -> VectorEndpointAccessError {
    VectorEndpointAccessError::FileDenied {
        vector_store_id: vector_store_id.to_owned(),
    }
}
