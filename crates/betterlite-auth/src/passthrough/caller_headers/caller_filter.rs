use std::collections::BTreeSet;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::dev_auth_mode::{AuthModeFlags, is_no_auth_dev_mode};
use crate::jwt_claims::TypedValue;
use crate::jwt_claims::decode::unverified_claims;
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::resolved_key::credential::safe_hash_api_key;

use super::credential_value::normalize_credential_value;
use super::header_sets::{LITELLM_INTERNAL_CLAIMS, MAPPED_ROUTE_CALLER_KEY_HEADER};
use super::header_text::header_text;

#[derive(Debug, Clone, Copy)]
pub struct FilterSettings<'settings> {
    pub master_key: Option<&'settings SecretString>,
    pub auth_mode: AuthModeFlags,
    pub custom_auth_configured: bool,
    pub litellm_key_header_name: Option<&'settings str>,
    pub endpoint_headers: &'settings [&'settings JsonObject],
}

// the override is added once, then the endpoint names deduplicate only among themselves
#[must_use]
pub fn operator_caller_key_names(settings: &FilterSettings<'_>) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(configured) = settings.litellm_key_header_name {
        names.push(configured.to_lowercase());
    }
    let mut endpoint_names: Vec<String> = Vec::new();
    for headers in settings.endpoint_headers {
        let Some(Json::Text(name)) = headers.get(MAPPED_ROUTE_CALLER_KEY_HEADER) else {
            continue;
        };
        let lowered = name.to_lowercase();
        if !endpoint_names.contains(&lowered) {
            endpoint_names.push(lowered);
        }
    }
    names.extend(endpoint_names);
    names
}

// pyjwt decodes without a signature check, so a value that is not a decodable compact JWT never matches
#[must_use]
pub fn is_authenticated_caller_jwt(value: &SecretString, claims: &Payload) -> bool {
    let Some(presented) = unverified_claims(value.expose_secret()) else {
        return false;
    };
    claims.0.iter().all(|(name, claim)| {
        LITELLM_INTERNAL_CLAIMS.contains(&name.as_str()) || claim_matches(&presented, name, claim)
    })
}

// dict.get on the presented claims: an absent claim reads as null, so a stored null matches it
fn claim_matches(presented: &TypedValue, name: &str, stored: &JsonData) -> bool {
    let Ok(stored_json) = serde_json::to_value(stored) else {
        return false;
    };
    presented
        .json
        .get(name)
        .map_or(stored_json.is_null(), |claim| *claim == stored_json)
}

#[must_use]
pub fn is_authenticated_caller_secret(
    value: &SecretString,
    auth: &UserAPIKeyAuth,
    settings: &FilterSettings<'_>,
) -> bool {
    let normalized = normalize_credential_value(value);
    let normalized_text = normalized.expose_secret();
    if let Some(master_key) = settings.master_key
        && bool::from(
            normalized_text
                .as_bytes()
                .ct_eq(master_key.expose_secret().as_bytes()),
        )
    {
        return true;
    }
    if let Some(claims) = &auth.extra.jwt_claims
        && !claims.0.is_empty()
        && is_authenticated_caller_jwt(&normalized, claims)
    {
        return true;
    }
    if is_no_auth_dev_mode(settings.master_key, &settings.auth_mode)
        && !settings.custom_auth_configured
    {
        return false;
    }
    let Some(stored_key) = &auth.extra.api_key.0 else {
        return false;
    };
    let stored_representation = safe_hash_api_key(normalized_text);
    bool::from(
        stored_representation
            .as_bytes()
            .ct_eq(stored_key.expose_secret().as_bytes()),
    )
}

// dict(request.headers): a name appears once, and Starlette's lookup keeps the first of a repeated header
#[must_use]
pub fn caller_headers_without_secrets(
    headers: &HeaderMap,
    auth: &UserAPIKeyAuth,
    never_forwarded: &[&str],
    settings: &FilterSettings<'_>,
) -> IndexMap<String, SecretString> {
    let mut incoming: IndexMap<String, SecretString> = IndexMap::new();
    for (name, value) in headers {
        incoming
            .entry(name.as_str().to_lowercase())
            .or_insert_with(|| SecretString::from(header_text(value)));
    }
    let mut dropped: BTreeSet<String> = never_forwarded
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    dropped.insert(MAPPED_ROUTE_CALLER_KEY_HEADER.to_owned());
    dropped.extend(operator_caller_key_names(settings));
    incoming
        .into_iter()
        .filter(|(name, value)| {
            !dropped.contains(name) && !is_authenticated_caller_secret(value, auth, settings)
        })
        .collect()
}
