#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_resource_grants::managed_rows::{
    ManagedRowsQuery, ManagedVectorStoreCache, ManagedVectorStoreRows, VECTOR_STORE_CACHE_PREFIX,
    VectorStoreRow, VectorStoreRowsFailure,
};
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::{UI_SESSION_TOKEN_TEAM_ID, UiSessionTeams};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::api::vector_store_access::VectorStoreAccess;
use crate::api::vector_store_access::auth::ObjectPermissionLookup;
use crate::api::vector_store_access::store::ManagedVectorStore;
use crate::api::vector_store_registry::store::VectorStoreRegistry;

use super::assertion::{assert_can_access_store_id, assert_can_access_vector_store};
use super::error::{
    ACCESS_DENIED_DETAIL, ManagedStoreAccessError, ManagedStorePolicyError,
    UNABLE_TO_VALIDATE_DETAIL,
};
use super::lookup::{ManagedStoreLookup, ManagedStoreRegistry, get_managed_vector_store};

const VS_A: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai"}"#;
const VS_A_TEAM_A: &str =
    r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","team_id":"a"}"#;
// a row carries every table column, so the record built from it keeps the unset columns as nulls
const ROW_A: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":null,"vector_store_description":null,"vector_store_metadata":null,"created_at":null,"updated_at":null,"litellm_credential_name":null,"litellm_params":null,"team_id":null,"user_id":null,"is_config":false}"#;
const ROW_A_TEAM_A: &str = r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":null,"vector_store_description":null,"vector_store_metadata":null,"created_at":null,"updated_at":null,"litellm_credential_name":null,"litellm_params":null,"team_id":"a","user_id":null,"is_config":false}"#;

fn record(fields: &str) -> LiteLlmManagedVectorStore {
    serde_json::from_str(fields).expect("the store fixture decodes")
}

fn store(fields: &str) -> ManagedVectorStore {
    serde_json::from_str(fields).expect("the managed store fixture decodes")
}

fn stored(fields: &str) -> ManagedVectorStoresTable {
    serde_json::from_str(fields).expect("the table row fixture decodes")
}

fn modelled(fields: &str) -> VectorStoreRow {
    let wrapped = format!(r#"{{"model_dump":{fields}}}"#);
    serde_json::from_str(&wrapped).expect("the fetched row fixture decodes")
}

fn payload_error() -> PayloadError {
    let invalid = serde_json::from_str::<i64>("not a number").unwrap_err();
    PayloadError::Json(invalid)
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct RegistryBroke(String);

#[derive(Default)]
struct ScriptedRegistry {
    stores: BTreeMap<String, LiteLlmManagedVectorStore>,
    failure: Option<String>,
    looks: Mutex<Vec<String>>,
}

impl ScriptedRegistry {
    fn with_store(vector_store_id: &str, fields: &str) -> Self {
        let mut registry = Self::default();
        registry
            .stores
            .insert(vector_store_id.to_owned(), record(fields));
        registry
    }

    fn failing(message: &str) -> Self {
        Self {
            stores: BTreeMap::new(),
            failure: Some(message.to_owned()),
            looks: Mutex::new(Vec::new()),
        }
    }

    fn looks(&self) -> Vec<String> {
        self.looks.lock().unwrap().clone()
    }
}

impl ManagedStoreRegistry for ScriptedRegistry {
    type Error = RegistryBroke;

    fn lookup(
        &self,
        vector_store_id: &str,
    ) -> Result<Option<LiteLlmManagedVectorStore>, RegistryBroke> {
        self.looks.lock().unwrap().push(vector_store_id.to_owned());
        match &self.failure {
            Some(message) => Err(RegistryBroke(message.clone())),
            None => Ok(self.stores.get(vector_store_id).cloned()),
        }
    }
}

#[derive(Default)]
struct ScriptedCache {
    hits: BTreeMap<String, ManagedVectorStoresTable>,
    fail_read: bool,
    fail_write: bool,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<String>>,
}

impl ScriptedCache {
    fn with_hit(vector_store_id: &str, fields: &str) -> Self {
        let mut cache = Self::default();
        cache.hits.insert(
            format!("{VECTOR_STORE_CACHE_PREFIX}{vector_store_id}"),
            stored(fields),
        );
        cache
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<String> {
        self.writes.lock().unwrap().clone()
    }
}

#[async_trait]
impl ManagedVectorStoreCache for ScriptedCache {
    async fn read(&self, key: &str) -> Result<Option<ManagedVectorStoresTable>, PayloadError> {
        self.reads.lock().unwrap().push(key.to_owned());
        if self.fail_read {
            return Err(payload_error());
        }
        Ok(self.hits.get(key).cloned())
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }

    async fn write(
        &self,
        key: &str,
        _row: &ManagedVectorStoresTable,
        _ttl: f64,
    ) -> Result<(), AuthCacheError> {
        if self.fail_write {
            return Err(AuthCacheError::from(payload_error()));
        }
        self.writes.lock().unwrap().push(key.to_owned());
        Ok(())
    }
}

#[derive(Default)]
struct ScriptedRows {
    rows: Vec<VectorStoreRow>,
    queries: Mutex<Vec<(Vec<String>, i64)>>,
}

impl ScriptedRows {
    fn queries(&self) -> Vec<(Vec<String>, i64)> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl ManagedVectorStoreRows for ScriptedRows {
    async fn by_uuids(
        &self,
        query: ManagedRowsQuery,
    ) -> Result<Vec<VectorStoreRow>, VectorStoreRowsFailure> {
        self.queries.lock().unwrap().push((query.uuids, query.take));
        Ok(self.rows.clone())
    }
}

#[derive(Default)]
struct World {
    database: bool,
    user_teams: Vec<String>,
    team_reads: Mutex<Vec<String>>,
}

impl World {
    fn team_reads(&self) -> Vec<String> {
        self.team_reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl ObjectPermissionLookup for World {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn load_permission(
        &self,
        _object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        Ok(None)
    }
}

#[async_trait]
impl TeamLoader for World {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.team_reads
            .lock()
            .unwrap()
            .push(request.team_id.to_owned());
        Err(TeamLookupError::TeamNotFound {
            team_id: request.team_id.to_owned(),
        })
    }
}

#[async_trait]
impl UiSessionTeams for World {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        Ok(Some(self.user_teams.clone()))
    }
}

fn access(world: &World) -> VectorStoreAccess<'_, World, World, World> {
    VectorStoreAccess {
        permissions: world,
        teams: world,
        session_teams: world,
    }
}

fn store_lookup<'lookup>(
    registry: Option<&'lookup ScriptedRegistry>,
    database: bool,
    cache: &'lookup ScriptedCache,
    rows: &'lookup ScriptedRows,
) -> ManagedStoreLookup<'lookup, ScriptedRegistry> {
    ManagedStoreLookup {
        registry,
        database,
        cache,
        rows,
    }
}

fn data_text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn data_permission(stores: &[&str]) -> JsonData {
    let listed = JsonData::Array(
        stores
            .iter()
            .map(|store_id| JsonData::String((*store_id).to_owned()))
            .collect(),
    );
    JsonData::Object(BTreeMap::from([
        ("object_permission_id".to_owned(), data_text("p_key")),
        ("vector_stores".to_owned(), listed),
    ]))
}

fn auth(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let map: BTreeMap<String, JsonData> = fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn caller(team_id: &str) -> UserAPIKeyAuth {
    auth(&[
        ("team_id", data_text(team_id)),
        ("user_id", data_text("u1")),
        ("user_role", data_text("internal_user")),
    ])
}

fn granted_caller(stores: &[&str]) -> UserAPIKeyAuth {
    auth(&[
        ("team_id", data_text("b")),
        ("user_id", data_text("u1")),
        ("user_role", data_text("internal_user")),
        ("object_permission", data_permission(stores)),
    ])
}

#[derive(Clone)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).expect("the captured log is UTF-8")
    }
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

async fn resolved(
    registry: Option<&ScriptedRegistry>,
    cache: &ScriptedCache,
    rows: &ScriptedRows,
    database: bool,
    vector_store_id: &str,
) -> (
    Result<Option<ManagedVectorStore>, ManagedStoreAccessError<RegistryBroke>>,
    String,
) {
    let (captured, guard) = capture();
    let lookup = store_lookup(registry, database, cache, rows);
    let outcome = get_managed_vector_store(&lookup, vector_store_id).await;
    drop(guard);
    (outcome, captured.text())
}

#[tokio::test]
async fn empty_id_skips_every_lookup() {
    let registry = ScriptedRegistry::with_store("vs_a", VS_A);
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(Some(&registry), &cache, &rows, true, "").await;
    assert_eq!(outcome.unwrap(), None);
    assert!(registry.looks().is_empty());
    assert!(cache.reads().is_empty());
    assert!(rows.queries().is_empty());
    assert!(logged.is_empty());
}

#[tokio::test]
async fn registry_hit_normalized() {
    let registry = ScriptedRegistry::with_store(
        "vs_a",
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","litellm_params":"{\"api_key\":\"k\"}"}"#,
    );
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(Some(&registry), &cache, &rows, true, "vs_a").await;
    assert_eq!(
        outcome.unwrap(),
        Some(store(
            r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","litellm_params":{"api_key":"k"}}"#
        ))
    );
    assert_eq!(registry.looks(), vec!["vs_a".to_owned()]);
    assert!(cache.reads().is_empty());
    assert!(cache.writes().is_empty());
    assert!(rows.queries().is_empty());
    assert!(logged.is_empty());
}

#[tokio::test]
async fn registry_hit_without_params_kept() {
    let registry = ScriptedRegistry::with_store("vs_a", VS_A);
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, _logged) = resolved(Some(&registry), &cache, &rows, true, "vs_a").await;
    assert_eq!(outcome.unwrap(), Some(store(VS_A)));
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn registry_non_object_params_normalized() {
    let registry = ScriptedRegistry::with_store(
        "vs_a",
        r#"{"vector_store_id":"vs_a","litellm_params":"[]"}"#,
    );
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, _logged) = resolved(Some(&registry), &cache, &rows, true, "vs_a").await;
    assert_eq!(
        outcome.unwrap(),
        Some(store(r#"{"vector_store_id":"vs_a","litellm_params":{}}"#))
    );
}

#[test]
fn real_registry_lookup_reads_loaded_stores() {
    let registry = VectorStoreRegistry {
        stores: vec![record(VS_A_TEAM_A)],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(CredentialSnapshot::default()),
    };
    let found = <VectorStoreRegistry as ManagedStoreRegistry>::lookup(&registry, "vs_a");
    assert_eq!(found.unwrap(), Some(record(VS_A_TEAM_A)));
    let missing = <VectorStoreRegistry as ManagedStoreRegistry>::lookup(&registry, "vs_missing");
    assert_eq!(missing.unwrap(), None);
}

#[tokio::test]
async fn registry_miss_resolves_from_database() {
    let registry = ScriptedRegistry::default();
    let cache = ScriptedCache::default();
    let mut rows = ScriptedRows::default();
    rows.rows.push(modelled(VS_A_TEAM_A));
    let (outcome, logged) = resolved(Some(&registry), &cache, &rows, true, "vs_a").await;
    assert_eq!(outcome.unwrap(), Some(store(ROW_A_TEAM_A)));
    assert_eq!(registry.looks(), vec!["vs_a".to_owned()]);
    assert_eq!(rows.queries(), vec![(vec!["vs_a".to_owned()], 1)]);
    assert_eq!(
        cache.reads(),
        vec![format!("{VECTOR_STORE_CACHE_PREFIX}vs_a")]
    );
    assert_eq!(
        cache.writes(),
        vec![format!("{VECTOR_STORE_CACHE_PREFIX}vs_a")]
    );
    assert!(logged.is_empty());
}

#[tokio::test]
async fn database_first_row_wins() {
    let cache = ScriptedCache::default();
    let mut rows = ScriptedRows::default();
    rows.rows.push(modelled(VS_A));
    rows.rows.push(modelled(
        r#"{"vector_store_id":"vs_b","custom_llm_provider":"openai"}"#,
    ));
    let (outcome, _logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    assert_eq!(outcome.unwrap(), Some(store(ROW_A)));
}

#[tokio::test]
async fn database_miss_returns_none() {
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(
        cache.reads(),
        vec![format!("{VECTOR_STORE_CACHE_PREFIX}vs_a")]
    );
    assert_eq!(rows.queries(), vec![(vec!["vs_a".to_owned()], 1)]);
    assert!(cache.writes().is_empty());
    assert!(logged.is_empty());
}

#[tokio::test]
async fn registry_miss_without_database() {
    let registry = ScriptedRegistry::default();
    let cache = ScriptedCache::default();
    let mut rows = ScriptedRows::default();
    rows.rows.push(modelled(VS_A));
    let (outcome, logged) = resolved(Some(&registry), &cache, &rows, false, "vs_a").await;
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(registry.looks(), vec!["vs_a".to_owned()]);
    assert!(cache.reads().is_empty());
    assert!(rows.queries().is_empty());
    assert!(logged.is_empty());
}

#[tokio::test]
async fn cache_hit_skips_row_query() {
    let cache = ScriptedCache::with_hit("vs_a", VS_A_TEAM_A);
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    assert_eq!(outcome.unwrap(), Some(store(ROW_A_TEAM_A)));
    assert_eq!(
        cache.reads(),
        vec![format!("{VECTOR_STORE_CACHE_PREFIX}vs_a")]
    );
    assert!(rows.queries().is_empty());
    assert!(cache.writes().is_empty());
    assert!(logged.is_empty());
}

#[tokio::test]
async fn registry_failure_fails_closed() {
    let registry = ScriptedRegistry::failing("registry broke");
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(Some(&registry), &cache, &rows, true, "vs_a").await;
    let error = outcome.unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(error.detail(), UNABLE_TO_VALIDATE_DETAIL);
    assert_eq!(
        error.to_string(),
        "Failed to resolve vector store id=vs_a from registry: registry broke"
    );
    assert!(
        logged.contains("Failed to resolve vector store id=vs_a from registry: registry broke")
    );
    assert!(cache.reads().is_empty());
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn cache_read_failure_fails_closed() {
    let cache = ScriptedCache {
        hits: BTreeMap::new(),
        fail_read: true,
        fail_write: false,
        reads: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    };
    let rows = ScriptedRows::default();
    let (outcome, logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    let error = outcome.unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(error.detail(), UNABLE_TO_VALIDATE_DETAIL);
    assert!(
        error
            .to_string()
            .starts_with("Failed to resolve vector store id=vs_a from shared cache: ")
    );
    assert!(logged.contains("from shared cache"));
    assert!(rows.queries().is_empty());
}

#[tokio::test]
async fn cache_write_failure_fails_closed() {
    let cache = ScriptedCache {
        hits: BTreeMap::new(),
        fail_read: false,
        fail_write: true,
        reads: Mutex::new(Vec::new()),
        writes: Mutex::new(Vec::new()),
    };
    let mut rows = ScriptedRows::default();
    rows.rows.push(modelled(VS_A));
    let (outcome, logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    let error = outcome.unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(error.detail(), UNABLE_TO_VALIDATE_DETAIL);
    assert!(
        error
            .to_string()
            .starts_with("Failed to resolve vector store id=vs_a from shared cache: ")
    );
    assert!(logged.contains("from shared cache"));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn unreadable_row_fails_closed() {
    let cache = ScriptedCache::default();
    let mut rows = ScriptedRows::default();
    rows.rows.push(modelled(
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","is_config":"yes"}"#,
    ));
    let (outcome, logged) = resolved(None, &cache, &rows, true, "vs_a").await;
    let error = outcome.unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(error.detail(), UNABLE_TO_VALIDATE_DETAIL);
    assert!(
        error
            .to_string()
            .starts_with("Failed to resolve vector store id=vs_a from shared cache: ")
    );
    assert!(logged.contains("from shared cache"));
    assert!(cache.writes().is_empty());
}

#[tokio::test]
async fn assert_teamless_store_allowed() {
    let world = World::default();
    let teamless = store(VS_A);
    assert_can_access_vector_store(&access(&world), &teamless, &caller("b"), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn assert_denied_default_detail() {
    let world = World::default();
    let owned = store(VS_A_TEAM_A);
    let error = assert_can_access_vector_store(&access(&world), &owned, &caller("b"), None)
        .await
        .unwrap_err();
    assert!(matches!(&error, ManagedStorePolicyError::Denied { .. }));
    assert_eq!(error.status(), 403);
    assert_eq!(error.detail(), ACCESS_DENIED_DETAIL);
}

#[tokio::test]
async fn assert_denied_supplied_detail() {
    let world = World::default();
    let owned = store(VS_A_TEAM_A);
    let error =
        assert_can_access_vector_store(&access(&world), &owned, &caller("b"), Some("blocked here"))
            .await
            .unwrap_err();
    assert_eq!(error.status(), 403);
    assert_eq!(error.detail(), "blocked here");
}

#[tokio::test]
async fn assert_inline_grant_allowed() {
    let world = World::default();
    let owned = store(VS_A_TEAM_A);
    assert_can_access_vector_store(&access(&world), &owned, &granted_caller(&["vs_a"]), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn assert_grant_for_other_store_denied() {
    let world = World::default();
    let owned = store(VS_A_TEAM_A);
    let error = assert_can_access_vector_store(
        &access(&world),
        &owned,
        &granted_caller(&["vs_other"]),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status(), 403);
    assert_eq!(error.detail(), ACCESS_DENIED_DETAIL);
}

#[tokio::test]
async fn assert_admin_allowed() {
    let world = World::default();
    let owned = store(VS_A_TEAM_A);
    let admin = auth(&[("user_role", data_text("proxy_admin"))]);
    assert_can_access_vector_store(&access(&world), &owned, &admin, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn assert_policy_failure_passes_through() {
    let world = World {
        database: true,
        user_teams: vec!["a".to_owned()],
        team_reads: Mutex::new(Vec::new()),
    };
    let session = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("user_role", data_text("internal_user")),
    ]);
    let owned = store(VS_A_TEAM_A);
    let error = assert_can_access_vector_store(&access(&world), &owned, &session, None)
        .await
        .unwrap_err();
    assert!(matches!(&error, ManagedStorePolicyError::Access(_)));
    assert_eq!(error.status(), 404);
    assert_eq!(
        error.detail(),
        "Team doesn't exist in db. Team=a. Create team via `/team/new` call."
    );
    assert_eq!(world.team_reads(), vec!["a".to_owned()]);
}

#[tokio::test]
async fn assert_id_unknown_passes() {
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let lookup = store_lookup(None, true, &cache, &rows);
    let world = World::default();
    let found =
        assert_can_access_store_id(&lookup, &access(&world), "vs_unknown", &caller("b"), None)
            .await
            .unwrap();
    assert_eq!(found, None);
}

#[tokio::test]
async fn assert_id_empty_passes() {
    let registry = ScriptedRegistry::with_store("vs_a", VS_A_TEAM_A);
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let lookup = store_lookup(Some(&registry), true, &cache, &rows);
    let world = World::default();
    let found = assert_can_access_store_id(&lookup, &access(&world), "", &caller("b"), None)
        .await
        .unwrap();
    assert_eq!(found, None);
    assert!(registry.looks().is_empty());
}

#[tokio::test]
async fn assert_id_managed_denied() {
    let registry = ScriptedRegistry::with_store("vs_a", VS_A_TEAM_A);
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let lookup = store_lookup(Some(&registry), true, &cache, &rows);
    let world = World::default();
    let error = assert_can_access_store_id(&lookup, &access(&world), "vs_a", &caller("b"), None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), 403);
    assert_eq!(error.detail(), ACCESS_DENIED_DETAIL);
    assert_eq!(error.to_string(), ACCESS_DENIED_DETAIL);
}

#[tokio::test]
async fn assert_id_managed_granted() {
    let registry = ScriptedRegistry::with_store(
        "vs_a",
        r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","team_id":"a","litellm_params":"{\"api_key\":\"k\"}"}"#,
    );
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let lookup = store_lookup(Some(&registry), true, &cache, &rows);
    let world = World::default();
    let found = assert_can_access_store_id(
        &lookup,
        &access(&world),
        "vs_a",
        &granted_caller(&["vs_a"]),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        found,
        Some(store(
            r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","team_id":"a","litellm_params":{"api_key":"k"}}"#
        ))
    );
}

#[tokio::test]
async fn assert_id_registry_failure_fails() {
    let registry = ScriptedRegistry::failing("registry broke");
    let cache = ScriptedCache::default();
    let rows = ScriptedRows::default();
    let lookup = store_lookup(Some(&registry), true, &cache, &rows);
    let world = World::default();
    let error = assert_can_access_store_id(&lookup, &access(&world), "vs_a", &caller("b"), None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), 500);
    assert_eq!(error.detail(), UNABLE_TO_VALIDATE_DETAIL);
    assert!(cache.reads().is_empty());
}
