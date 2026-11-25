use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_core::api::managed_store_access::assertion::{
    assert_can_access_store_id, assert_can_access_vector_store,
};
use betterlite_core::api::managed_store_access::error::ManagedStoreAccessError;
use betterlite_core::api::managed_store_access::lookup::get_managed_vector_store;
use betterlite_core::api::vector_store_access::VectorStoreAccess;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_access::store::ManagedVectorStore;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::world::{RegistryBroke, World};
use super::{Call, StoreAuth};

// the credential-side lookups the policy can reach, scripted empty: the cases carry grants inline
struct NoLookups;

#[async_trait]
impl ObjectPermissionLookup for NoLookups {
    fn has_database(&self) -> bool {
        false
    }

    async fn load_permission(
        &self,
        _object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl TeamLoader for NoLookups {
    async fn load_team(
        &self,
        _request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        Err(TeamLookupError::NoDatabase)
    }
}

#[async_trait]
impl UiSessionTeams for NoLookups {
    fn has_database(&self) -> bool {
        false
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        Ok(None)
    }
}

fn insert_text(map: &mut BTreeMap<String, JsonData>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(String::from(name), JsonData::String(String::from(value)));
    }
}

// the inline grant the key carries, built only when the case scripts one of its fields
fn permission(fields: &StoreAuth) -> JsonData {
    let mut object = BTreeMap::new();
    insert_text(
        &mut object,
        "object_permission_id",
        fields.object_permission_id.as_deref(),
    );
    if let Some(stores) = &fields.vector_stores {
        let listed = stores
            .iter()
            .map(|store_id| JsonData::String(store_id.clone()))
            .collect();
        object.insert(String::from("vector_stores"), JsonData::Array(listed));
    }
    JsonData::Object(object)
}

fn key_auth(fields: &StoreAuth) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut map = BTreeMap::new();
    insert_text(&mut map, "team_id", fields.team_id.as_deref());
    insert_text(&mut map, "user_id", fields.user_id.as_deref());
    insert_text(&mut map, "user_role", fields.user_role.as_deref());
    if fields.object_permission_id.is_some() || fields.vector_stores.is_some() {
        map.insert(String::from("object_permission"), permission(fields));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).or_value_error()
}

fn store_view(store: Option<&ManagedVectorStore>) -> Result<Value, Box<HelperError>> {
    match store {
        Some(found) => serde_json::to_value(&found.0).or_value_error(),
        None => Ok(Value::Null),
    }
}

// which warning line the failure emitted, named by the arm the way the case names it
fn warning_labels(failure: &ManagedStoreAccessError<RegistryBroke>) -> Vec<&'static str> {
    match failure {
        ManagedStoreAccessError::RegistryFailure { .. } => vec!["registry"],
        ManagedStoreAccessError::SharedCacheFailure { .. } => vec!["shared_cache"],
        ManagedStoreAccessError::Policy(_) => Vec::new(),
    }
}

fn answer_view(
    function: &str,
    store: Value,
    error: Option<Value>,
    warnings: Vec<&str>,
    lookups: Value,
) -> Value {
    json!({
        "function": function,
        "store": store,
        "error": error,
        "warnings": warnings,
        "lookups": lookups,
    })
}

fn resolution_answer(
    runtime: &Runtime,
    world: &World,
    vector_store_id: &str,
) -> Result<Value, Box<HelperError>> {
    let lookup = world.lookup();
    let outcome = runtime.block_on(get_managed_vector_store(&lookup, vector_store_id));
    let (store, error, warnings) = match &outcome {
        Ok(found) => (store_view(found.as_ref())?, None, Vec::new()),
        Err(failure) => (
            Value::Null,
            Some(json!({"status": failure.status(), "detail": failure.detail()})),
            warning_labels(failure),
        ),
    };
    Ok(answer_view(
        "resolve",
        store,
        error,
        warnings,
        world.lookups(),
    ))
}

fn assert_store_answer(
    runtime: &Runtime,
    store: &LiteLlmManagedVectorStore,
    auth: &StoreAuth,
    detail: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let key = key_auth(auth)?;
    let no_lookups = NoLookups;
    let access = VectorStoreAccess {
        permissions: &no_lookups,
        teams: &no_lookups,
        session_teams: &no_lookups,
    };
    let managed = ManagedVectorStore(store.clone());
    let outcome = runtime.block_on(assert_can_access_vector_store(
        &access, &managed, &key, detail,
    ));
    let error = match &outcome {
        Ok(()) => None,
        Err(failure) => Some(json!({"status": failure.status(), "detail": failure.detail()})),
    };
    let lookups = json!({
        "registry": [],
        "cache_reads": [],
        "cache_writes": [],
        "row_queries": [],
    });
    Ok(answer_view(
        "assert_store",
        Value::Null,
        error,
        Vec::new(),
        lookups,
    ))
}

fn assert_id_answer(
    runtime: &Runtime,
    world: &World,
    vector_store_id: &str,
    auth: &StoreAuth,
    detail: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let key = key_auth(auth)?;
    let no_lookups = NoLookups;
    let access = VectorStoreAccess {
        permissions: &no_lookups,
        teams: &no_lookups,
        session_teams: &no_lookups,
    };
    let lookup = world.lookup();
    let outcome = runtime.block_on(assert_can_access_store_id(
        &lookup,
        &access,
        vector_store_id,
        &key,
        detail,
    ));
    let (store, error, warnings) = match &outcome {
        Ok(found) => (store_view(found.as_ref())?, None, Vec::new()),
        Err(failure) => (
            Value::Null,
            Some(json!({"status": failure.status(), "detail": failure.detail()})),
            warning_labels(failure),
        ),
    };
    Ok(answer_view(
        "assert_id",
        store,
        error,
        warnings,
        world.lookups(),
    ))
}

pub(super) fn answer(runtime: &Runtime, call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Resolve {
            vector_store_id,
            registry,
            database,
            cached,
            rows,
            fail,
        } => {
            let world = World::build(registry.as_ref(), *database, cached, rows, *fail)?;
            resolution_answer(runtime, &world, vector_store_id)
        }
        Call::AssertStore {
            store,
            auth,
            detail,
        } => assert_store_answer(runtime, store, auth, detail.as_deref()),
        Call::AssertId {
            vector_store_id,
            registry,
            database,
            cached,
            rows,
            fail,
            auth,
            detail,
        } => {
            let world = World::build(registry.as_ref(), *database, cached, rows, *fail)?;
            assert_id_answer(runtime, &world, vector_store_id, auth, detail.as_deref())
        }
    }
}
