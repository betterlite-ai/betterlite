use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_cache::copy::copy_for_cache;
use betterlite_auth::auth_cache::keys::{
    CacheKey, END_USER_RESTRICTED_OVERFLOW, GROUP_REGISTRY_OVERFLOW, NO_TEAM_MEMBERSHIP, Partition,
    TAG_REGISTRY_OVERFLOW, USER_NO_MCP_PERMISSION, partition,
};
use betterlite_auth::auth_cache::payload;
use betterlite_auth::auth_cache::{AuthCache, gateway_cache};
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::codec::CodecError;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

#[derive(Deserialize)]
struct KeyIds {
    user_id: String,
    object_permission_id: String,
    tag: String,
    access_group: String,
    project_id: String,
    end_user_id: String,
    team_id: String,
    member_user_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "scenario")]
pub(crate) enum Input {
    Codec {
        payloads: Vec<Json>,
    },
    ManagementTtl {
        ttls: Vec<Option<f64>>,
        fallback: f64,
    },
    Partition {
        keys: Vec<String>,
    },
    Keys {
        ids: KeyIds,
    },
    Copy {
        auth: BTreeMap<String, JsonData>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Codec { payloads } => codec(payloads),
            Input::ManagementTtl { ttls, fallback } => management_ttl(ttls, *fallback),
            Input::Partition { keys } => partition_report(keys),
            Input::Keys { ids } => Ok(keys(ids)),
            Input::Copy { auth } => copy(auth),
        }
    }
}

enum Failure {
    Payload(payload::PayloadError),
    Auth(AuthError),
    Memory,
    Encoding(serde_json::Error),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let (class, message) = match failure {
            Failure::Payload(error) => ("ValueError", error.to_string()),
            Failure::Auth(error) => ("ValueError", error.to_string()),
            Failure::Memory => ("OSError", "the local memory tier did not build".to_owned()),
            Failure::Encoding(error) => ("ValueError", error.to_string()),
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    }
}

fn codec(payloads: &[Json]) -> HelperResult {
    let mut projections = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let untyped = payload::encode(payload).map_err(Failure::Payload)?;
        let (typed, error) =
            match payload::encode_validated::<LiteLLMObjectPermissionTable>(payload) {
                Ok(typed) => (Some(typed), None),
                Err(payload::PayloadError::Codec(CodecError::Validation(_source))) => {
                    (None, Some("validation"))
                }
                Err(payload::PayloadError::Codec(CodecError::Serialize(_source))) => {
                    (None, Some("serialize"))
                }
                Err(other) => return Err(Box::<HelperError>::from(Failure::Payload(other))),
            };
        let decoded = match payload::decode::<LiteLLMObjectPermissionTable>(payload)
            .map_err(Failure::Payload)?
        {
            Some(row) => Some(serde_json::to_value(row).map_err(Failure::Encoding)?),
            None => None,
        };
        projections.push(json!({
            "typed": typed,
            "error": error,
            "untyped": untyped,
            "decoded": decoded,
        }));
    }
    Ok(json!({ "projections": projections }))
}

fn management_ttl(ttls: &[Option<f64>], fallback: f64) -> HelperResult {
    let mut configured = Vec::with_capacity(ttls.len());
    for ttl in ttls {
        let cache = local_auth_cache(*ttl, fallback)?;
        configured.push(cache.management_object_ttl());
    }
    Ok(json!({ "ttls": configured }))
}

fn partition_report(keys: &[String]) -> HelperResult {
    let cache = local_auth_cache(None, 60.0_f64)?;
    let key_store = cache.in_memory_cache_for(&"0".repeat(64));
    let mut selections = Vec::with_capacity(keys.len());
    for key in keys {
        selections.push(json!({
            "key": key,
            "key_partition": partition(key) == Partition::KeyObject,
            "store": Arc::ptr_eq(cache.in_memory_cache_for(key), key_store),
        }));
    }
    Ok(json!({ "selections": selections }))
}

fn keys(ids: &KeyIds) -> serde_json::Value {
    json!({
        "user_object_permission_id": CacheKey::UserObjectPermissionId(&ids.user_id).render(),
        "object_permission_id": CacheKey::ObjectPermissionId(&ids.object_permission_id).render(),
        "tag": CacheKey::Tag(&ids.tag).render(),
        "tag_registry": CacheKey::TagRegistry.render(),
        "tag_registry_overflow": TAG_REGISTRY_OVERFLOW,
        "model_access_group": CacheKey::ModelAccessGroup(&ids.access_group).render(),
        "model_access_group_registry": CacheKey::ModelAccessGroupRegistry.render(),
        "model_access_group_registry_overflow": GROUP_REGISTRY_OVERFLOW,
        "model_access_group_spend": CacheKey::ModelAccessGroupSpend(&ids.access_group).render(),
        "project_id": CacheKey::ProjectId(&ids.project_id).render(),
        "spend_project": CacheKey::ProjectSpend(&ids.project_id).render(),
        "end_user_id": CacheKey::EndUserId(&ids.end_user_id).render(),
        "end_user_restricted_registry": CacheKey::EndUserRestrictedRegistry.render(),
        "end_user_restricted_registry_overflow": END_USER_RESTRICTED_OVERFLOW,
        "team_membership_auth": CacheKey::TeamMembershipAuth {
            team_id: &ids.team_id,
            user_id: &ids.member_user_id,
        }
        .render(),
        "team_membership_reservation": CacheKey::TeamMembershipReservation {
            user_id: &ids.member_user_id,
            team_id: &ids.team_id,
        }
        .render(),
        "user_no_mcp_permission": USER_NO_MCP_PERMISSION,
        "no_team_membership": NO_TEAM_MEMBERSHIP,
    })
}

fn copy(fields: &BTreeMap<String, JsonData>) -> HelperResult {
    let original =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).map_err(Failure::Auth)?;
    let copied = copy_for_cache(&original);
    let original_dump = serde_json::to_value(&original).map_err(Failure::Encoding)?;
    let copied_dump = serde_json::to_value(&copied).map_err(Failure::Encoding)?;
    Ok(json!({
        "dump_equal": copied_dump == original_dump,
    }))
}

fn local_auth_cache(
    memory_ttl: Option<f64>,
    fallback: f64,
) -> Result<AuthCache<DualCache<LocalMemoryOps>>, Box<HelperError>> {
    let settings = DualCacheSettings {
        default_in_memory_ttl: memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    Ok(gateway_cache(
        local_memory()?,
        local_memory()?,
        None,
        settings,
        fallback,
    ))
}

fn local_memory() -> Result<Arc<LocalMemoryOps>, Box<HelperError>> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 0.0_f64);
    LocalMemoryOps::new(1024, clock)
        .map(Arc::new)
        .map_err(|_source| Box::<HelperError>::from(Failure::Memory))
}
