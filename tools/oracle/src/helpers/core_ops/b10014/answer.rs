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
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
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
    if let Some(text) = value {
        map.insert(String::from(name), JsonData::String(String::from(text)));
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
    store.map_or(Ok(Value::Null), |found| {
        serde_json::to_value(&found.0).or_value_error()
    })
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
    store: &Value,
    error: Option<&Value>,
    warnings: &[&str],
    lookups: &Value,
) -> Value {
    json!({
        "function": function,
        "store": store,
        "error": error,
        "warnings": warnings,
        "lookups": lookups,
    })
}

// the store, error and warnings one outcome answers with, whichever call produced it
fn outcome_answer(
    function: &str,
    outcome: Result<Option<ManagedVectorStore>, ManagedStoreAccessError<RegistryBroke>>,
    lookups: &Value,
) -> Result<Value, Box<HelperError>> {
    match outcome {
        Ok(found) => {
            let store = store_view(found.as_ref())?;
            Ok(answer_view(function, &store, None, &[], lookups))
        }
        Err(failure) => {
            let error = json!({"status": failure.status(), "detail": failure.detail()});
            Ok(answer_view(
                function,
                &Value::Null,
                Some(&error),
                &warning_labels(&failure),
                lookups,
            ))
        }
    }
}

fn resolution_answer(
    runtime: &Runtime,
    world: &World,
    vector_store_id: &str,
) -> Result<Value, Box<HelperError>> {
    let lookup = world.lookup();
    let outcome = runtime.block_on(get_managed_vector_store(&lookup, vector_store_id));
    outcome_answer("resolve", outcome, &world.lookups())
}

// a store the case holds is asserted without a world, so no library lookup can run
fn no_library_lookups() -> Value {
    json!({
        "registry": [],
        "cache_reads": [],
        "cache_writes": [],
        "row_queries": [],
    })
}

// which assertion an answer runs: the store the case holds, or the store an id resolves to
enum Assertion<'assertion> {
    Store(&'assertion LiteLlmManagedVectorStore),
    Id {
        world: &'assertion World,
        vector_store_id: &'assertion str,
    },
}

// both assertions answer the same shape, so one function runs whichever the case asked for
fn assertion_answer(
    runtime: &Runtime,
    subject: &Assertion<'_>,
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
    let (function, outcome, lookups) = match subject {
        Assertion::Store(store) => {
            let managed = ManagedVectorStore((*store).clone());
            let outcome = runtime
                .block_on(assert_can_access_vector_store(
                    &access, &managed, &key, detail,
                ))
                .map(|()| None)
                .map_err(ManagedStoreAccessError::from);
            ("assert_store", outcome, no_library_lookups())
        }
        Assertion::Id {
            world,
            vector_store_id,
        } => {
            let lookup = world.lookup();
            let outcome = runtime.block_on(assert_can_access_store_id(
                &lookup,
                &access,
                vector_store_id,
                &key,
                detail,
            ));
            ("assert_id", outcome, world.lookups())
        }
    };
    outcome_answer(function, outcome, &lookups)
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
        } => {
            let subject = Assertion::Store(store.as_ref());
            assertion_answer(runtime, &subject, auth, detail.as_deref())
        }
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
            let subject = Assertion::Id {
                world: &world,
                vector_store_id: vector_store_id.as_str(),
            };
            assertion_answer(runtime, &subject, auth, detail.as_deref())
        }
    }
}
