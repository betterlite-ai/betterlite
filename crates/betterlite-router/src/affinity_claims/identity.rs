use std::collections::{BTreeSet, IndexMap};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use sha2::{Digest as _, Sha256};

use super::error::AffinityError;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_value_store::RouteKey;

pub const CACHE_KEY_PREFIX: &str = "deployment_affinity:v1";
pub const USER_ID_AFFINITY_PREFIX: &str = "user_id:";
pub const SESSION_ID_GENERATED_KEY: &str = "litellm_session_id_generated";
pub const SESSION_DEPLOYMENT_AFFINITY_TTL_KEY: &str = "_session_deployment_affinity_ttl";
pub const LOG_VALUE_KEEP: usize = 8;

const UNSCOPED_CALLER: &str = "unscoped";

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AffinityKey(pub(crate) RouteKey);

impl AffinityKey {
    pub fn parse(raw: &str) -> Result<Self, AffinityError> {
        let key = RouteKey::parse(raw)?;
        Ok(Self(key))
    }
}

impl AsRef<str> for AffinityKey {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

// int(value, 16) with the length check LiteLLM layers over it, in ASCII hex digits
#[must_use]
pub fn looks_like_sha256_hex(value: &str) -> bool {
    match value.len() {
        64 => value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        _ => false,
    }
}

#[must_use]
pub fn hash_user_key(user_key: &str) -> String {
    if looks_like_sha256_hex(user_key) {
        return user_key.to_ascii_lowercase();
    }
    let digest = Sha256::digest(user_key.as_bytes());
    hex::encode(digest)
}

pub fn affinity_cache_key(model_group: &str, user_key: &str) -> Result<AffinityKey, AffinityError> {
    let hashed_user_key = hash_user_key(user_key);
    let raw = format!("{CACHE_KEY_PREFIX}:{model_group}:{hashed_user_key}");
    AffinityKey::parse(&raw)
}

pub fn session_affinity_cache_key(
    model_group: &str,
    session_id: &str,
    user_key: Option<&str>,
) -> Result<AffinityKey, AffinityError> {
    let hashed_user_key = match user_key {
        Some(user_key) => hash_user_key(user_key),
        None => UNSCOPED_CALLER.to_owned(),
    };
    let raw = format!("{CACHE_KEY_PREFIX}:session:{model_group}:{hashed_user_key}:{session_id}");
    AffinityKey::parse(&raw)
}

fn metadata_dicts(request_kwargs: &JsonObject) -> Vec<&IndexMap<String, Json>> {
    let mut dicts = Vec::with_capacity(2);
    for channel in ["litellm_metadata", "metadata"] {
        if let Some(Json::Object(metadata)) = request_kwargs.get(channel) {
            dicts.push(metadata);
        }
    }
    dicts
}

fn first_metadata_value(
    request_kwargs: &JsonObject,
    field: &'static str,
) -> Result<Option<String>, AffinityError> {
    for metadata in metadata_dicts(request_kwargs) {
        match metadata.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Text(text)) => return Ok(Some(text.clone())),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(AffinityError::MetadataNotText { field });
            }
        }
    }
    Ok(None)
}

pub fn user_key_from_request_kwargs(
    request_kwargs: &JsonObject,
) -> Result<Option<String>, AffinityError> {
    if let Some(hash) = first_metadata_value(request_kwargs, "user_api_key_hash")? {
        return Ok(Some(hash));
    }
    let Some(user_id) = first_metadata_value(request_kwargs, "user_api_key_user_id")? else {
        return Ok(None);
    };
    Ok(Some(format!("{USER_ID_AFFINITY_PREFIX}{user_id}")))
}

fn session_id_from_metadata_dict(
    metadata: &IndexMap<String, Json>,
) -> Result<Option<String>, AffinityError> {
    let Some(session_id) = metadata.get("session_id") else {
        return Ok(None);
    };
    if matches!(session_id, Json::Null) {
        return Ok(None);
    }
    match metadata.get(SESSION_ID_GENERATED_KEY) {
        None | Some(Json::Null | Json::Bool(false)) => {}
        Some(Json::Bool(true)) => return Ok(None),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AffinityError::MetadataNotBool {
                field: SESSION_ID_GENERATED_KEY,
            });
        }
    }
    match session_id {
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(AffinityError::MetadataNotText {
                field: "session_id",
            })
        }
    }
}

pub fn session_id_from_request_kwargs(
    request_kwargs: &JsonObject,
) -> Result<Option<String>, AffinityError> {
    for metadata in metadata_dicts(request_kwargs) {
        if let Some(session_id) = session_id_from_metadata_dict(metadata)? {
            return Ok(Some(session_id));
        }
    }
    Ok(None)
}

fn positive_integer(value: &Json) -> Option<u64> {
    match value {
        Json::Number(number) => number.as_u64().filter(|value| *value > 0),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub fn marker_session_affinity_ttl(
    request_kwargs: &JsonObject,
) -> Result<Option<u32>, AffinityError> {
    for metadata in metadata_dicts(request_kwargs) {
        let Some(value) = metadata.get(SESSION_DEPLOYMENT_AFFINITY_TTL_KEY) else {
            continue;
        };
        if let Some(ttl) = positive_integer(value) {
            let seconds = u32::try_from(ttl).map_err(|_error| AffinityError::TtlOutOfRange {
                field: SESSION_DEPLOYMENT_AFFINITY_TTL_KEY,
            })?;
            return Ok(Some(seconds));
        }
    }
    Ok(None)
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AffinityFlag {
    Deployment,
    ResponsesApi,
    Session,
}

impl AffinityFlag {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "deployment_affinity" => Some(Self::Deployment),
            "responses_api_deployment_check" => Some(Self::ResponsesApi),
            "session_affinity" => Some(Self::Session),
            _ => None,
        }
    }
}

#[must_use]
pub fn effective_flags(
    config: Option<&IndexMap<String, Vec<String>>>,
    model_group: &str,
    defaults: &BTreeSet<AffinityFlag>,
) -> BTreeSet<AffinityFlag> {
    let Some(flags) = config.and_then(|entries| entries.get(model_group)) else {
        return defaults.clone();
    };
    flags
        .iter()
        .filter_map(|flag| AffinityFlag::from_name(flag.as_str()))
        .collect()
}

#[must_use]
pub fn model_map_key_from_litellm_model_name(litellm_model_name: &str) -> Option<String> {
    if litellm_model_name.is_empty() {
        return None;
    }
    let Some((provider_prefix, remainder)) = litellm_model_name.split_once('/') else {
        return Some(litellm_model_name.to_owned());
    };
    if provider_prefix == "azure" {
        return None;
    }
    Some(remainder.to_owned())
}

#[must_use]
pub fn model_map_key_from_deployment(deployment: &DeploymentSpec) -> Option<String> {
    if !deployment.model_name.is_empty() {
        return Some(deployment.model_name.clone());
    }
    if let Some(Json::Text(base_model)) = deployment.model_info.get("base_model")
        && !base_model.is_empty()
    {
        return Some(base_model.clone());
    }
    if let Some(Json::Text(base_model)) = deployment.litellm_params.get("base_model")
        && !base_model.is_empty()
    {
        return Some(base_model.clone());
    }
    if let Some(Json::Text(litellm_model_name)) = deployment.litellm_params.get("model")
        && !litellm_model_name.is_empty()
    {
        return model_map_key_from_litellm_model_name(litellm_model_name);
    }
    None
}

#[must_use]
pub fn stable_model_map_key(deployments: &[DeploymentSpec]) -> Option<String> {
    let mut keys = BTreeSet::new();
    for deployment in deployments {
        let key = model_map_key_from_deployment(deployment)?;
        keys.insert(key);
    }
    if keys.len() != 1 {
        return None;
    }
    keys.first().cloned()
}

#[must_use]
pub fn shorten_for_logs(value: &str) -> String {
    if value.chars().count() <= LOG_VALUE_KEEP {
        return value.to_owned();
    }
    let mut shortened: String = value.chars().take(LOG_VALUE_KEEP).collect();
    shortened.push_str("...");
    shortened
}

#[must_use]
pub fn find_deployment_by_model_id<'a>(
    deployments: &'a [DeploymentSpec],
    model_id: &str,
) -> Option<&'a DeploymentSpec> {
    for deployment in deployments {
        if deployment.id() == Some(model_id) {
            return Some(deployment);
        }
    }
    None
}
