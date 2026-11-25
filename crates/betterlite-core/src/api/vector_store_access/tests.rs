#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

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
use serde::de::DeserializeOwned;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

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
    world: &World,
    vector_stores: &[ManagedVectorStore],
    caller: &UserAPIKeyAuth,
) -> Vec<String> {
    let access = policy(world);
    let allowed = access
        .filter_listable_vector_stores(vector_stores, caller)
        .await
        .unwrap();
    let mut ids = Vec::with_capacity(allowed.len());
    for vector_store in allowed {
        ids.push(vector_store.0.vector_store_id.clone().unwrap_or_default());
    }
    ids
}

fn permission_object(object_permission_id: &str, stores: &[&str]) -> Json {
    let allowed = stores.iter().copied().map(text).collect();
    object(&[
        ("object_permission_id", text(object_permission_id)),
        ("vector_stores", Json::List(allowed)),
    ])
}

fn wire<T>(value: &Json) -> T
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_string(value).unwrap();
    serde_json::from_str(&encoded).unwrap()
}

fn permission_row(object_permission_id: &str, stores: &[&str]) -> LiteLLMObjectPermissionTable {
    wire(&permission_object(object_permission_id, stores))
}

fn team_row(
    team_id: &str,
    permission_id: Option<&str>,
    inline: Option<(&str, &[&str])>,
) -> LiteLLMTeamTableCachedObj {
    let permission = inline.map_or(Json::Null, |(id, stores)| permission_object(id, stores));
    let fields = [
        ("team_id", text(team_id)),
        (
            "object_permission_id",
            permission_id.map_or(Json::Null, text),
        ),
        ("object_permission", permission),
    ];
    wire(&object(&fields))
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
    wire(&object(fields))
}

fn data_text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn data_permission(object_permission_id: &str, stores: &[&str]) -> JsonData {
    data_object(&[
        ("object_permission_id", data_text(object_permission_id)),
        (
            "vector_stores",
            JsonData::Array(stores.iter().copied().map(data_text).collect()),
        ),
    ])
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

fn dashboard_world() -> World {
    let mut world = World::with_database();
    world.user_teams = vec!["a".to_owned(), "b".to_owned()];
    world.teams.insert(
        "a".to_owned(),
        team_row("a", None, Some(("a_perm", &["vs_a"]))),
    );
    world
        .teams
        .insert("b".to_owned(), team_row("b", Some("p_b"), None));
    world
}

fn session_caller() -> UserAPIKeyAuth {
    auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("object_permission", data_permission("p_key", &["vs_key"])),
        (
            "parent_otel_span",
            data_object(&[("span_id", data_text("s1"))]),
        ),
    ])
}

fn assert_params_read_as_empty(raw: &str) {
    let encoded = store(&[("litellm_params", text(raw))]);
    assert_eq!(
        normalize_litellm_params(&encoded).unwrap(),
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_debug_logs() -> (Output, DefaultGuard) {
    let output = Output(Arc::new(Mutex::new(Vec::new())));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(Level::DEBUG)
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
        normalize_litellm_params(&original).unwrap(),
        store(&[
            ("vector_store_id", text("vs_a")),
            ("litellm_params", object(&[("api_key", text("sk-secret"))])),
        ])
    );
    assert_params_read_as_empty("[]");
    assert_params_read_as_empty("null");
    assert_params_read_as_empty("1");
    assert_params_read_as_empty("broken");
    let parsed = store(&[("litellm_params", object(&[("api_key", text("k"))]))]);
    assert_eq!(normalize_litellm_params(&parsed).unwrap(), parsed);
    let null_params = store(&[("litellm_params", Json::Null)]);
    assert_eq!(normalize_litellm_params(&null_params).unwrap(), null_params);
    let absent = store(&[("vector_store_id", text("vs_a"))]);
    assert_eq!(normalize_litellm_params(&absent).unwrap(), absent);
    let flagged = store(&[("litellm_params", Json::Bool(true))]);
    assert_eq!(normalize_litellm_params(&flagged).unwrap(), flagged);
}

#[test]
fn nesting_past_the_json_limit_normalizes_to_an_empty_object() {
    let deep = format!("{}1{}", "{\"a\":".repeat(127), "}".repeat(127));
    let encoded = store(&[("litellm_params", text(&deep))]);
    let normalized = normalize_litellm_params(&encoded).unwrap();
    assert!(
        normalized
            .0
            .litellm_params
            .as_ref()
            .is_some_and(|params| params.get("a").is_some())
    );
    assert_params_read_as_empty(&format!("{}1{}", "{\"a\":".repeat(129), "}".repeat(129)));
}

#[test]
fn debug_redacts_stored_and_normalized_credentials() {
    let stored = store(&[
        ("vector_store_id", text("vs_a")),
        ("litellm_params", text(r#"{"api_key":"sk-secret"}"#)),
        ("provider_token", text("sk-extra-secret")),
    ]);
    let shown = format!("{stored:?}");
    assert!(shown.contains("vs_a"));
    assert!(shown.contains("present"));
    assert!(shown.contains("provider_token"));
    assert!(!shown.contains("sk-secret"));
    assert!(!shown.contains("sk-extra-secret"));
    let normalized = normalize_litellm_params(&stored).unwrap();
    assert!(
        normalized
            .0
            .litellm_params
            .as_ref()
            .is_some_and(|params| params.get("api_key").is_some())
    );
    let normalized_shown = format!("{normalized:?}");
    assert!(normalized_shown.contains("present"));
    assert!(!normalized_shown.contains("sk-secret"));
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
    let numeric_team = store(&[
        ("vector_store_id", text("vs_e")),
        ("team_id", Json::Number(serde_json::Number::from(5_i32))),
    ]);
    let caller = auth(&[("team_id", data_text("u"))]);
    assert!(granted(&access, &teamless, &caller).await);
    assert!(granted(&access, &null_team, &caller).await);
    assert!(!granted(&access, &empty_team, &caller).await);
    assert!(!granted(&access, &owned, &caller).await);
    assert!(!granted(&access, &numeric_team, &caller).await);
    let numeric_caller = auth(&[("team_id", data_text("5"))]);
    assert!(!granted(&access, &numeric_team, &numeric_caller).await);
    let stores = [teamless, null_team, empty_team, owned];
    assert!(listable(&world, &stores, &caller).await.is_empty());
    let empty_team_caller = auth(&[("team_id", data_text(""))]);
    assert!(granted(&access, &stores[2], &empty_team_caller).await);
    assert_eq!(
        listable(&world, &stores, &empty_team_caller).await,
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
        listable(&world, &stores, &admin).await,
        vec!["vs_a".to_owned(), "vs_b".to_owned()]
    );
    let session_admin = auth(&[
        ("team_id", data_text(UI_SESSION_TOKEN_TEAM_ID)),
        ("user_id", data_text("u1")),
        ("user_role", data_text("proxy_admin")),
    ]);
    assert_eq!(listable(&world, &stores, &session_admin).await.len(), 2);
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
    assert!(listable(&world, &stores, &viewer).await.is_empty());
}

#[tokio::test]
async fn dashboard_effective_teams() {
    let mut world = dashboard_world();
    world.allow("p_b", &["vs_b"]);
    let access = policy(&world);
    let caller = session_caller();
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
        listable(&world, &stores, &caller).await,
        vec!["vs_key".to_owned(), "vs_a".to_owned(), "vs_b".to_owned()]
    );
    assert!(!granted(&access, &stores[3], &caller).await);
    assert!(!granted(&access, &stores[4], &caller).await);
}

#[tokio::test]
async fn dashboard_contexts_clear_and_replace_team_fields() {
    let world = dashboard_world();
    let access = policy(&world);
    let caller = session_caller();
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
    assert_eq!(
        contexts[1]
            .team_object_permission
            .as_ref()
            .unwrap()
            .object_permission_id,
        "a_perm"
    );
    assert_eq!(contexts[2].team_object_permission, None);
    assert!(contexts.iter().all(|context| {
        context
            .object_permission
            .as_ref()
            .is_some_and(|permission| permission.object_permission_id == "p_key")
    }));
    let carried = format!("{:?}", contexts[2].credential);
    assert!(carried.contains("s1"));
    assert!(carried.contains("u1"));
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
    assert!(format!("{:?}", contexts[0].credential).contains("u1"));
}

#[test]
fn auth_context_keeps_credential_secrets_out_of_its_fields() {
    let secret = "plain-secret-value";
    let first = auth(&[
        ("api_key", data_text(secret)),
        ("team_id", data_text("t")),
        ("user_id", data_text("u1")),
    ]);
    let second = auth(&[
        ("api_key", data_text("other-secret")),
        ("team_id", data_text("t")),
        ("user_id", data_text("u1")),
    ]);
    let context = AuthContext::from_auth(&first).unwrap();
    let rendered = format!("{context:?}");
    assert!(!rendered.contains(secret));
    assert_eq!(context.team_id.as_deref(), Some("t"));
    assert_eq!(context.object_permission, None);
    assert_eq!(context.object_permission_id, None);
    assert_eq!(context.team_object_permission, None);
    assert_eq!(context.team_object_permission_id, None);
    assert_eq!(
        format!("{:?}", AuthContext::from_auth(&second).unwrap()),
        rendered
    );
}
