#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::{UI_SESSION_TOKEN_TEAM_ID, UiSessionTeams};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use serde_json::json;
use tracing::subscriber::set_default;

use super::VectorStoreAccess;
use super::auth::{AuthContext, ObjectPermissionLookup};
use super::decode::decode_vector_store_access;
use super::error::VectorStoreAccessError;
use super::store::{ManagedVectorStore, normalize_litellm_params};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

#[derive(Default)]
struct World {
    database: bool,
    permissions: BTreeMap<String, LiteLLMObjectPermissionTable>,
    failing: BTreeSet<String>,
    teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    user_teams: Vec<String>,
    read_permissions: Mutex<Vec<String>>,
    read_teams: Mutex<Vec<String>>,
}

impl World {
    fn with_database() -> Self {
        Self {
            database: true,
            permissions: BTreeMap::new(),
            failing: BTreeSet::new(),
            teams: BTreeMap::new(),
            user_teams: Vec::new(),
            read_permissions: Mutex::new(Vec::new()),
            read_teams: Mutex::new(Vec::new()),
        }
    }

    fn allow(&mut self, object_permission_id: &str, stores: &[&str]) {
        self.permissions.insert(
            object_permission_id.to_owned(),
            permission_row(object_permission_id, stores),
        );
    }

    fn fail(&mut self, object_permission_id: &str) {
        self.failing.insert(object_permission_id.to_owned());
    }

    fn permission_reads(&self) -> Vec<String> {
        self.read_permissions.lock().unwrap().clone()
    }

    fn team_reads(&self) -> Vec<String> {
        self.read_teams.lock().unwrap().clone()
    }
}

#[async_trait]
impl ObjectPermissionLookup for World {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn load_permission(
        &self,
        object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        self.read_permissions
            .lock()
            .unwrap()
            .push(object_permission_id.to_owned());
        if self.failing.contains(object_permission_id) {
            return Err(PermissionError::MissingInDatabaseOnly);
        }
        Ok(self.permissions.get(object_permission_id).cloned())
    }
}

#[async_trait]
impl TeamLoader for World {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.read_teams
            .lock()
            .unwrap()
            .push(request.team_id.to_owned());
        self.teams
            .get(request.team_id)
            .cloned()
            .ok_or_else(|| TeamLookupError::TeamNotFound {
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

fn policy(world: &World) -> VectorStoreAccess<'_, World, World, World> {
    VectorStoreAccess {
        permissions: world,
        teams: world,
        session_teams: world,
    }
}

async fn granted(
    access: &VectorStoreAccess<'_, World, World, World>,
    vector_store: &ManagedVectorStore,
    caller: &UserAPIKeyAuth,
) -> bool {
    access
        .can_user_access_vector_store(vector_store, caller)
        .await
        .unwrap()
}

async fn listable(
    access: &VectorStoreAccess<'_, World, World, World>,
    vector_stores: &[ManagedVectorStore],
    caller: &UserAPIKeyAuth,
) -> Vec<String> {
    let list = access
        .filter_listable_vector_stores(vector_stores, caller)
        .await
        .unwrap();
    list.iter()
        .map(|vector_store| vector_store.grant_id().to_owned())
        .collect()
}

fn permission_row(object_permission_id: &str, stores: &[&str]) -> LiteLLMObjectPermissionTable {
    serde_json::from_value(json!({
        "object_permission_id": object_permission_id,
        "vector_stores": stores,
    }))
    .unwrap()
}

fn team_row(
    team_id: &str,
    permission_id: Option<&str>,
    inline: Option<(&str, &[&str])>,
) -> LiteLLMTeamTableCachedObj {
    let permission =
        inline.map(|(id, stores)| json!({"object_permission_id": id, "vector_stores": stores}));
    serde_json::from_value(json!({
        "team_id": team_id,
        "object_permission_id": permission_id,
        "object_permission": permission,
    }))
    .unwrap()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(fields: &[(&str, Json)]) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn store(fields: &[(&str, Json)]) -> ManagedVectorStore {
    ManagedVectorStore(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn data_text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn data_permission(object_permission_id: &str, stores: &[&str]) -> JsonData {
    serde_json::from_value(json!({
        "object_permission_id": object_permission_id,
        "vector_stores": stores,
    }))
    .unwrap()
}

fn data_object(fields: &[(&str, JsonData)]) -> JsonData {
    JsonData::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

fn auth(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let map: BTreeMap<String, JsonData> = fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

fn assert_params_read_as_empty(raw: &str) {
    let encoded = store(&[("litellm_params", text(raw))]);
    assert_eq!(
        normalize_litellm_params(&encoded),
        store(&[("litellm_params", object(&[]))])
    );
    assert_eq!(
        decode_vector_store_access(raw.as_bytes(), ApiLimits::default()).unwrap(),
        object(&[])
    );
}

#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Output {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug_logs() -> (Output, tracing::subscriber::DefaultGuard) {
    let output = Output(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (output, set_default(subscriber))
}

#[test]
fn parameter_string_normalization() {
    let original = store(&[
        ("vector_store_id", text("vs_a")),
        ("litellm_params", text(r#"{"api_key":"sk-secret"}"#)),
    ]);
    assert_eq!(
        normalize_litellm_params(&original),
        store(&[
            ("vector_store_id", text("vs_a")),
            ("litellm_params", object(&[("api_key", text("sk-secret"))])),
        ])
    );
    assert!(!format!("{original:?}").contains("sk-secret"));
    assert!(!format!("{original:?}").contains("api_key"));
    assert!(format!("{original:?}").contains("litellm_params: \"text\""));
    assert_params_read_as_empty("[]");
    assert_params_read_as_empty("null");
    assert_params_read_as_empty("1");
    assert_params_read_as_empty("broken");
    let parsed = store(&[("litellm_params", object(&[("api_key", text("k"))]))]);
    assert_eq!(normalize_litellm_params(&parsed), parsed);
    assert!(format!("{parsed:?}").contains("litellm_params: \"object\""));
    let null_params = store(&[("litellm_params", Json::Null)]);
    assert_eq!(normalize_litellm_params(&null_params), null_params);
    assert!(format!("{null_params:?}").contains("litellm_params: \"other\""));
    let absent = store(&[("vector_store_id", text("vs_a"))]);
    assert_eq!(normalize_litellm_params(&absent), absent);
    assert!(format!("{absent:?}").contains("litellm_params: \"absent\""));
    let numeric = store(&[(
        "litellm_params",
        Json::Number(serde_json::Number::from(5_u64)),
    )]);
    assert_eq!(normalize_litellm_params(&numeric), numeric);
}

#[test]
fn decoder_reads_only_an_object_within_the_json_limit() {
    let limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    assert_eq!(
        decode_vector_store_access(br#"{"":""}"#, limits).unwrap(),
        object(&[("", text(""))])
    );
    assert_eq!(
        decode_vector_store_access(br#"{"a":""}"#, limits).unwrap(),
        object(&[("a", text(""))])
    );
    let rejected = decode_vector_store_access(br#"{"ab":""}"#, limits).unwrap_err();
    assert!(matches!(
        rejected,
        VectorStoreAccessError::Limits(ApiLimitError::JsonTooLarge {
            observed: 9,
            limit: 8
        })
    ));
    assert_eq!(
        decode_vector_store_access(b"broken", limits).unwrap(),
        object(&[])
    );
    assert_eq!(
        decode_vector_store_access(b"[]", limits).unwrap(),
        object(&[])
    );
    assert_eq!(
        decode_vector_store_access(&[0xff], limits).unwrap(),
        object(&[])
    );
}

#[test]
fn non_standard_json_tokens_normalize_to_an_empty_object() {
    // Python's json module takes these tokens; standard JSON and serde_json do not
    assert_params_read_as_empty(r#"{"a": NaN}"#);
    assert_params_read_as_empty(r#"{"b": Infinity}"#);
    assert_params_read_as_empty(r#"{"c": "\ud800"}"#);
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/vector_store_access");
    let replayed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            drop(decode_vector_store_access(&bytes, ApiLimits::default()));
        })
        .count();
    assert!(replayed >= 10);
}

#[tokio::test]
async fn store_grant_precedence() {
    let mut world = World::with_database();
    world.allow("p_key", &["vs_a"]);
    world.fail("p_team_fail");
    let access = policy(&world);
    let target = store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]);
    let other = store(&[("vector_store_id", text("vs_a")), ("team_id", text("u"))]);
    let teammate = auth(&[("team_id", data_text("t"))]);
    assert!(granted(&access, &target, &teammate).await);
    assert!(!granted(&access, &other, &teammate).await);
    let key_granted = auth(&[
        ("team_id", data_text("u")),
        ("object_permission", data_permission("p_key", &["vs_a"])),
        ("team_object_permission_id", data_text("p_team_fail")),
    ]);
    assert!(granted(&access, &target, &key_granted).await);
    let team_granted = auth(&[
        ("team_id", data_text("u")),
        (
            "team_object_permission",
            data_permission("q_team", &["vs_a"]),
        ),
    ]);
    assert!(granted(&access, &target, &team_granted).await);
    let by_key_id = auth(&[
        ("team_id", data_text("u")),
        ("object_permission_id", data_text("p_key")),
    ]);
    assert!(granted(&access, &target, &by_key_id).await);
    assert_eq!(world.permission_reads(), vec!["p_key".to_owned()]);
}

#[tokio::test]
async fn user_id_grants_nothing() {
    let world = World::with_database();
    let access = policy(&world);
    let target = store(&[
        ("vector_store_id", text("vs_a")),
        ("team_id", text("t")),
        ("user_id", text("u1")),
    ]);
    let caller = auth(&[("user_id", data_text("u1")), ("team_id", data_text("u"))]);
    assert!(!granted(&access, &target, &caller).await);
}

#[tokio::test]
async fn inline_permissions_hide_their_id_rows() {
    let mut world = World::with_database();
    world.allow("p_hidden", &["vs_a"]);
    let access = policy(&world);
    let target = store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]);
    let caller = auth(&[
        ("team_id", data_text("u")),
        ("object_permission", data_permission("p_hidden", &[])),
        ("object_permission_id", data_text("p_hidden")),
        ("team_object_permission", data_permission("q_hidden", &[])),
        ("team_object_permission_id", data_text("q_hidden")),
    ]);
    assert!(!granted(&access, &target, &caller).await);
    assert!(world.permission_reads().is_empty());
}

#[tokio::test]
async fn falsey_ids_and_missing_database_return_no_permission() {
    let mut world = World::with_database();
    world.allow("p_key", &["vs_a"]);
    let access = policy(&world);
    let target = store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]);
    let empty_id = auth(&[
        ("team_id", data_text("u")),
        ("object_permission_id", data_text("")),
    ]);
    let null_id = auth(&[
        ("team_id", data_text("u")),
        ("object_permission_id", JsonData::Null),
    ]);
    assert!(!granted(&access, &target, &empty_id).await);
    assert!(!granted(&access, &target, &null_id).await);
    assert!(world.permission_reads().is_empty());
    let without_database = World::default();
    let no_database_access = policy(&without_database);
    let by_key_id = auth(&[
        ("team_id", data_text("u")),
        ("object_permission_id", data_text("p_key")),
    ]);
    assert!(!granted(&no_database_access, &target, &by_key_id).await);
    assert!(without_database.permission_reads().is_empty());
}

#[tokio::test]
async fn lookup_failures_fall_through_to_later_grants() {
    let mut world = World::with_database();
    world.fail("p_fail");
    let (output, guard) = capture_debug_logs();
    let access = policy(&world);
    let target = store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]);
    let caller = auth(&[
        ("team_id", data_text("u")),
        ("object_permission_id", data_text("p_fail")),
        (
            "team_object_permission",
            data_permission("q_team", &["vs_a"]),
        ),
    ]);
    assert!(granted(&access, &target, &caller).await);
    let teammate = auth(&[
        ("team_id", data_text("t")),
        ("object_permission_id", data_text("p_fail")),
    ]);
    assert!(granted(&access, &target, &teammate).await);
    drop(guard);
    assert!(
        output
            .text()
            .contains("Failed to load object_permission id=p_fail:")
    );
    assert!(
        output
            .text()
            .contains("Referenced object permission does not exist")
    );
}

#[tokio::test]
async fn teamless_access_is_not_listing() {
    let world = World::with_database();
    let access = policy(&world);
    let teamless = store(&[("vector_store_id", text("vs_a"))]);
    let null_team = store(&[("vector_store_id", text("vs_b")), ("team_id", Json::Null)]);
    let empty_team = store(&[("vector_store_id", text("vs_c")), ("team_id", text(""))]);
    let owned = store(&[("vector_store_id", text("vs_d")), ("team_id", text("t"))]);
    let caller = auth(&[("team_id", data_text("u"))]);
    assert!(granted(&access, &teamless, &caller).await);
    assert!(granted(&access, &null_team, &caller).await);
    assert!(!granted(&access, &empty_team, &caller).await);
    assert!(!granted(&access, &owned, &caller).await);
    let stores = [teamless, null_team, empty_team, owned];
    assert!(listable(&access, &stores, &caller).await.is_empty());
    let empty_team_caller = auth(&[("team_id", data_text(""))]);
    assert!(granted(&access, &stores[2], &empty_team_caller).await);
    assert_eq!(
        listable(&access, &stores, &empty_team_caller).await,
        vec!["vs_c".to_owned()]
    );
}

#[tokio::test]
async fn admin_access_and_listing_skip_every_lookup() {
    let mut world = World::with_database();
    world.fail("p_fail");
    world.user_teams = vec!["missing".to_owned()];
    let access = policy(&world);
    let stores = [
        store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]),
        store(&[("vector_store_id", text("vs_b"))]),
    ];
    let admin = auth(&[
        ("user_role", data_text("proxy_admin")),
        ("object_permission_id", data_text("p_fail")),
    ]);
    assert!(granted(&access, &stores[0], &admin).await);
    assert_eq!(
        listable(&access, &stores, &admin).await,
        vec!["vs_a".to_owned(), "vs_b".to_owned()]
    );
    let session_admin = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("user_role", data_text("proxy_admin")),
    ]);
    assert_eq!(listable(&access, &stores, &session_admin).await.len(), 2);
    assert!(world.permission_reads().is_empty());
    assert!(world.team_reads().is_empty());
}

#[tokio::test]
async fn proxy_admin_viewer_is_not_an_admin() {
    let world = World::with_database();
    let access = policy(&world);
    let stores = [
        store(&[("vector_store_id", text("vs_a")), ("team_id", text("t"))]),
        store(&[("vector_store_id", text("vs_b"))]),
    ];
    let viewer = auth(&[
        ("team_id", data_text("u")),
        ("user_role", data_text("proxy_admin_viewer")),
    ]);
    assert!(!granted(&access, &stores[0], &viewer).await);
    assert!(listable(&access, &stores, &viewer).await.is_empty());
}

#[tokio::test]
async fn dashboard_effective_teams() {
    let mut world = World::with_database();
    world.user_teams = vec!["a".to_owned(), "b".to_owned()];
    world.teams.insert(
        "a".to_owned(),
        team_row("a", None, Some(("a_perm", &["vs_a"]))),
    );
    world.allow("p_b", &["vs_b"]);
    world
        .teams
        .insert("b".to_owned(), team_row("b", Some("p_b"), None));
    let access = policy(&world);
    let caller = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("object_permission", data_permission("p_key", &["vs_key"])),
        (
            "parent_otel_span",
            data_object(&[("span_id", data_text("s1"))]),
        ),
    ]);
    let stores = [
        store(&[("vector_store_id", text("vs_key")), ("team_id", text("t"))]),
        store(&[("vector_store_id", text("vs_a")), ("team_id", text("a"))]),
        store(&[("vector_store_id", text("vs_b")), ("team_id", text("t"))]),
        store(&[
            ("vector_store_id", text("vs_s")),
            ("team_id", text(UI_SESSION_TOKEN_TEAM_ID)),
        ]),
        store(&[("vector_store_id", text("vs_x")), ("team_id", text("z"))]),
    ];
    assert_eq!(
        listable(&access, &stores, &caller).await,
        vec!["vs_key".to_owned(), "vs_a".to_owned(), "vs_b".to_owned()]
    );
    assert!(!granted(&access, &stores[3], &caller).await);
    assert!(!granted(&access, &stores[4], &caller).await);
}

#[tokio::test]
async fn dashboard_contexts_clear_and_replace_team_fields() {
    let mut world = World::with_database();
    world.user_teams = vec!["a".to_owned(), "b".to_owned()];
    world.teams.insert(
        "a".to_owned(),
        team_row("a", None, Some(("a_perm", &["vs_a"]))),
    );
    world
        .teams
        .insert("b".to_owned(), team_row("b", Some("p_b"), None));
    let access = policy(&world);
    let caller = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("object_permission", data_permission("p_key", &["vs_key"])),
        (
            "parent_otel_span",
            data_object(&[("span_id", data_text("s1"))]),
        ),
    ]);
    let contexts = access.vector_store_auth_contexts(&caller).await.unwrap();
    assert_eq!(contexts.len(), 3);
    assert_eq!(contexts[0].team_id, None);
    assert_eq!(contexts[0].team_object_permission, None);
    assert_eq!(contexts[0].team_object_permission_id, None);
    let key_permission = contexts[0].object_permission.as_ref().unwrap();
    assert_eq!(key_permission.object_permission_id, "p_key");
    assert_eq!(contexts[1].team_id.as_deref(), Some("a"));
    assert_eq!(contexts[1].team_object_permission_id, None);
    assert_eq!(contexts[2].team_id.as_deref(), Some("b"));
    assert_eq!(
        contexts[2].team_object_permission_id.as_deref(),
        Some("p_b")
    );
    assert_eq!(contexts[1].fields.get("team_id"), Some(&data_text("a")));
    assert_eq!(
        contexts[1].fields.get("team_object_permission_id"),
        Some(&JsonData::Null)
    );
    assert_eq!(
        contexts[2].fields.get("team_object_permission_id"),
        Some(&data_text("p_b"))
    );
    let span = data_object(&[("span_id", data_text("s1"))]);
    assert!(
        contexts
            .iter()
            .all(|context| context.fields.get("parent_otel_span") == Some(&span))
    );
    assert!(
        contexts
            .iter()
            .all(|context| context.fields.get("user_id") == Some(&data_text("u1")))
    );
}

#[tokio::test]
async fn empty_team_resolution_keeps_the_key_context() {
    let mut world = World::with_database();
    world.user_teams = vec![String::new()];
    let access = policy(&world);
    let caller = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("object_permission", data_permission("p_key", &["vs_key"])),
    ]);
    let granted_store = store(&[("vector_store_id", text("vs_key")), ("team_id", text("t"))]);
    let owned = store(&[("vector_store_id", text("vs_a")), ("team_id", text("a"))]);
    assert!(granted(&access, &granted_store, &caller).await);
    assert!(!granted(&access, &owned, &caller).await);
    assert_eq!(
        access
            .vector_store_auth_contexts(&caller)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(world.team_reads().is_empty());
}

#[tokio::test]
async fn team_lookup_errors_propagate_as_not_found() {
    let mut world = World::with_database();
    world.user_teams = vec!["missing".to_owned()];
    let access = policy(&world);
    let caller = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
    ]);
    let target = store(&[
        ("vector_store_id", text("vs_a")),
        ("team_id", text("missing")),
    ]);
    let error = access
        .can_user_access_vector_store(&target, &caller)
        .await
        .unwrap_err();
    assert_eq!(error.http(), Some(404));
    let stores = [target];
    let listing_error = access
        .filter_listable_vector_stores(&stores, &caller)
        .await
        .unwrap_err();
    assert_eq!(listing_error.http(), Some(404));
}

#[tokio::test]
async fn non_session_credentials_keep_their_context() {
    let world = World::with_database();
    let access = policy(&world);
    let caller = auth(&[("team_id", data_text("t")), ("user_id", data_text("u1"))]);
    let contexts = access.vector_store_auth_contexts(&caller).await.unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].team_id.as_deref(), Some("t"));
    assert_eq!(contexts[0].fields.get("user_id"), Some(&data_text("u1")));
}

#[test]
fn auth_context_debug_names_fields_without_values() {
    let caller = auth(&[
        ("team_id", data_text("t")),
        ("user_id", data_text("u1")),
        ("object_permission", data_permission("p_key", &["vs_a"])),
    ]);
    let context = AuthContext::from_auth(&caller).unwrap();
    let rendered = format!("{context:?}");
    assert_eq!(context.team_id.as_deref(), Some("t"));
    assert!(rendered.contains("field_count:"));
    assert!(!rendered.contains("u1"));
    assert!(!rendered.contains("api_key"));
}
