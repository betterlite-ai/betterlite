use std::str::FromStr as _;

use betterlite_auth::route_match::canonical::{RequestRouteSource, ScopeFields};
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::vector_endpoint_access::error::VectorEndpointAccessError;
use betterlite_core::api::vector_endpoint_access::{
    VectorEndpointAccess, VectorEndpointTables, file_endpoint_decision,
};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_registration::JsonObject;

use crate::api::provider_hints::{provider_from_body, provider_from_headers, provider_from_query};

use super::{Caller, KeyFacts, RequestFacts, has_content};

// _resolve_provider then _maybe_check_permissions: the provider a request names, and B1061's check when either index allowlist is set.
pub fn provider_and_index_permissions(
    data: &JsonObject,
    vector_store_id: &str,
    facts: &RequestFacts<'_>,
    caller: &Caller<'_>,
    tables: &dyn VectorEndpointTables,
) -> Result<Option<ProviderId>, VectorEndpointAccessError> {
    let Some(provider) = resolve_provider(data, facts) else {
        return Ok(None);
    };
    let key: KeyFacts = serde_json::to_value(caller.auth)
        .and_then(serde_json::from_value)
        .map_err(VectorEndpointAccessError::Grants)?;
    let gated = key.metadata.as_ref().is_some_and(index_allowlist)
        || key.team_metadata.as_ref().is_some_and(index_allowlist);
    if !gated {
        return Ok(Some(provider));
    }
    let access = VectorEndpointAccess {
        provider,
        vector_store_id,
        method: facts.method,
        route: route_source(facts),
        auth: caller.auth,
        tables,
    };
    file_endpoint_decision(&access)?;
    Ok(Some(provider))
}

// The value the source chain chose, kept whole so only a text value converts to a provider.
enum ProviderCandidate<'candidate> {
    Data(&'candidate Json),
    Body(&'candidate JsonValue),
    Text(String),
}

// data.custom_llm_provider, the header, the query, the body for a write, then openai; empty and unknown values are null.
fn resolve_provider(data: &JsonObject, facts: &RequestFacts<'_>) -> Option<ProviderId> {
    let mut provider = source_chain(data, facts);
    if provider.is_none() && matches!(facts.method, "POST" | "PUT" | "PATCH") {
        provider = provider_from_body(facts.body)
            .filter(|value| !value.is_null())
            .map(ProviderCandidate::Body);
    }
    let chosen = provider.unwrap_or_else(|| ProviderCandidate::Text(String::from("openai")));
    provider_id(&chosen)
}

// custom_llm_provider or the header or the query: Python's or chain keeps the last operand when all earlier ones are falsy.
fn source_chain<'chain>(
    data: &'chain JsonObject,
    facts: &'chain RequestFacts<'_>,
) -> Option<ProviderCandidate<'chain>> {
    if let Some(value) = data.get("custom_llm_provider")
        && has_content(value)
    {
        return Some(ProviderCandidate::Data(value));
    }
    if let Some(header) = provider_from_headers(facts.headers).filter(|header| !header.is_empty()) {
        return Some(ProviderCandidate::Text(header));
    }
    provider_from_query(facts.query).map(ProviderCandidate::Text)
}

// LlmProviders(provider): only a string that names a provider converts.
fn provider_id(candidate: &ProviderCandidate<'_>) -> Option<ProviderId> {
    let text = match candidate {
        ProviderCandidate::Data(Json::Text(text)) => Some(text.as_str()),
        ProviderCandidate::Data(
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
        ) => None,
        ProviderCandidate::Body(JsonValue::String(text)) => Some(text.as_str()),
        ProviderCandidate::Body(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        ) => None,
        ProviderCandidate::Text(text) => Some(text.as_str()),
    };
    text.and_then(|text| ProviderId::from_str(text).ok())
}

// metadata.get("allowed_vector_store_indexes") in Python truthiness: null and the empty list do not gate.
fn index_allowlist(metadata: &JsonObject) -> bool {
    metadata
        .get("allowed_vector_store_indexes")
        .is_some_and(has_content)
}

// RequestRouteSource has no Clone, so B1061 gets the same scope pieces and path rebuilt.
fn route_source<'route>(facts: &RequestFacts<'route>) -> RequestRouteSource<'route> {
    RequestRouteSource {
        scope: facts.route.scope.as_ref().map(|scope| ScopeFields {
            path: scope.path,
            app_root_path: scope.app_root_path,
            root_path: scope.root_path,
        }),
        url_path: facts.route.url_path,
    }
}
