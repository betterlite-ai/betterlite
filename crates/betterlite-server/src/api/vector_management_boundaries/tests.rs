#![cfg(test)]

mod postgres;

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use betterlite_auth::auth_prefetch::permission::lookup::PermissionError;
use betterlite_auth::auth_team_lookup::error::{TeamFailure, TeamLookupError};
use betterlite_auth::auth_team_lookup::lookup::TeamRequest;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::ui_session_identity::{UI_SESSION_TOKEN_TEAM_ID, UiSessionTeams};
use betterlite_core::api::api_boundaries::{ApiLimitError, ApiLimits};
use betterlite_core::api::vector_store_access::VectorStoreAccess;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_core::api::vector_store_registry::store::VectorStoreRegistry;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::managed_resource_rows::files::ManagedVectorStoresTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::error::{
    ACCESS_DENIED, ConfigOwnedDetail, ErrorDetail, VectorManagementDetail, VectorManagementError,
};
use super::fetch::{check_vector_store_access, fetch_store_before_access};
use super::json::{ManagementJsonError, decode_management_json};
use super::ownership::{raise_if_config_owned, with_ownership};
use super::redaction::{REDACTED_BY_LITELM, redact_sensitive_litellm_params, redact_store_params};
use super::rows::{VectorStoreRows, row_to_vector_store};

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn rendered(record: &LiteLlmManagedVectorStore) -> Json {
    serde_json::to_value(record)
        .and_then(serde_json::from_value)
        .unwrap()
}

fn parsed_record(source: &str) -> LiteLlmManagedVectorStore {
    serde_json::to_value(json(source))
        .and_then(serde_json::from_value)
        .unwrap()
}

fn registry(stored: Vec<LiteLlmManagedVectorStore>) -> VectorStoreRegistry {
    let mut registry = VectorStoreRegistry::default();
    for store in stored {
        registry.add_store(store);
    }
    registry
}

fn redacted_text(params: &str, depth: usize) -> Json {
    redact_sensitive_litellm_params(&Json::Text(params.to_owned()), depth, ApiLimits::default())
}

const REDACT_CASE: &str = r#"{
    "api_key": null,
    "API-Key": "secret",
    "connection_string": "secret",
    "vertex_credentials": {},
    "api_base": "https://api.example.com/v1",
    "region": "us",
    "model": "text-embedding-3-small",
    "api_version": "2024-02-01",
    "max_tokens": 256,
    "input_cost_per_token": 0.0000001,
    "litellm_embedding_config": {
        "api_key": "secret",
        "api_base": "https://api.example.com/v1"
    }
}"#;

const REDACTED_CASE: &str = r#"{
    "api_key": "REDACTED_BY_LITELM",
    "API-Key": "REDACTED_BY_LITELM",
    "connection_string": "REDACTED_BY_LITELM",
    "vertex_credentials": "REDACTED_BY_LITELM",
    "api_base": "https://api.example.com/v1",
    "region": "us",
    "model": "text-embedding-3-small",
    "api_version": "2024-02-01",
    "max_tokens": 256,
    "input_cost_per_token": 0.0000001,
    "litellm_embedding_config": {
        "api_key": "REDACTED_BY_LITELM",
        "api_base": "https://api.example.com/v1"
    }
}"#;

#[test]
fn ownership_projection_supplies_absent_is_config_as_false() {
    let original = parsed_record(r#"{"vector_store_id":"vs_a"}"#);
    let owned = with_ownership(&original);
    assert_eq!(owned.is_config, Some(false));
    assert_eq!(original.is_config, None);
    assert!(!original.extra.contains_key("is_config"));
    assert_eq!(
        rendered(&owned),
        json(r#"{"vector_store_id":"vs_a","is_config":false}"#)
    );
}

#[test]
fn ownership_projection_keeps_present_true_false_and_null() {
    let configured = with_ownership(&parsed_record(
        r#"{"vector_store_id":"vs_a","is_config":true}"#,
    ));
    assert_eq!(configured.is_config, Some(true));
    assert_eq!(
        rendered(&configured),
        json(r#"{"vector_store_id":"vs_a","is_config":true}"#)
    );
    let database = with_ownership(&parsed_record(
        r#"{"vector_store_id":"vs_a","is_config":false}"#,
    ));
    assert_eq!(database.is_config, Some(false));
    let null = with_ownership(&parsed_record(
        r#"{"vector_store_id":"vs_a","is_config":null}"#,
    ));
    assert_eq!(null.is_config, None);
    assert_eq!(
        rendered(&null),
        json(r#"{"vector_store_id":"vs_a","is_config":null}"#)
    );
}

#[test]
fn raise_if_config_owned_allows_an_absent_registry() {
    raise_if_config_owned("vs_a", None).unwrap();
}

#[test]
fn raise_if_config_owned_allows_a_non_config_id() {
    let configured = registry(vec![parsed_record(
        r#"{"vector_store_id":"vs_a","is_config":true}"#,
    )]);
    raise_if_config_owned("vs_b", Some(&configured)).unwrap();
    let database_owned = registry(vec![parsed_record(r#"{"vector_store_id":"vs_b"}"#)]);
    raise_if_config_owned("vs_b", Some(&database_owned)).unwrap();
}

#[test]
fn raise_if_config_owned_reports_the_config_detail() {
    let configured = registry(vec![parsed_record(
        r#"{"vector_store_id":"vs_a","is_config":true}"#,
    )]);
    let error = raise_if_config_owned("vs_a", Some(&configured)).unwrap_err();
    let (status, detail) = error.http().unwrap();
    assert_eq!(status, 400);
    assert_eq!(
        detail,
        VectorManagementDetail::ConfigOwned(ConfigOwnedDetail {
            error: "Vector store vs_a is defined in the config file, so the config file owns it and it cannot be changed here. Edit the config file to change it, or remove it from the file to let the database own it.".to_owned(),
            vector_store_id: "vs_a".to_owned(),
        })
    );
}

#[test]
fn http_details_cover_only_the_helpers_http_exceptions() {
    assert_eq!(
        VectorManagementError::Rows(TableError::MissingRow).http(),
        None
    );
    let parse_error = serde_json::from_str::<u8>("").unwrap_err();
    assert_eq!(
        VectorManagementError::Access(VectorStoreAccessError::Fields(parse_error)).http(),
        None
    );
    assert_eq!(
        VectorManagementError::Access(VectorStoreAccessError::Team(
            TeamLookupError::AccessPolicyUnavailable {
                access_group_id: "ag_a".to_owned(),
                cause: Box::new(TeamFailure::Throttled),
            }
        ))
        .http(),
        Some((
            503,
            VectorManagementDetail::Text("Access group policy is unavailable".to_owned())
        ))
    );
    assert_eq!(
        VectorManagementError::Access(VectorStoreAccessError::Team(TeamLookupError::NoDatabase))
            .http(),
        None
    );
}

#[test]
fn redact_vector_credentials_marks_sensitive_values() {
    assert_eq!(
        redact_sensitive_litellm_params(&json(REDACT_CASE), 0, ApiLimits::default()),
        json(REDACTED_CASE)
    );
}

#[test]
fn redact_vector_credentials_consumes_depth_per_nested_dict() {
    let nested = json(r#"{"outer": {"inner": "kept"}}"#);
    assert_eq!(
        redact_sensitive_litellm_params(&nested, 8, ApiLimits::default()),
        json(r#"{"outer": {"inner": "kept"}}"#)
    );
    assert_eq!(
        redact_sensitive_litellm_params(&nested, 9, ApiLimits::default()),
        json(r#"{"outer": "REDACTED_BY_LITELM"}"#)
    );
    assert_eq!(
        redact_sensitive_litellm_params(&nested, 10, ApiLimits::default()),
        json(r#""REDACTED_BY_LITELM""#)
    );
}

#[test]
fn redact_vector_credentials_passes_top_level_scalars_and_plain_lists_through() {
    for source in [
        "null",
        "7",
        "1.5",
        "true",
        "false",
        r#"["a", 7, null, ["nested"]]"#,
    ] {
        assert_eq!(
            redact_sensitive_litellm_params(&json(source), 0, ApiLimits::default()),
            json(source)
        );
    }
}

#[test]
fn redact_vector_credentials_redacts_credential_maps_inside_lists() {
    let params = json(r#"{"configs": [{"api_key": "secret", "region": "us"}, "plain"]}"#);
    assert_eq!(
        redact_sensitive_litellm_params(&params, 0, ApiLimits::default()),
        json(r#"{"configs": [{"api_key": "REDACTED_BY_LITELM", "region": "us"}, "plain"]}"#)
    );
}

#[test]
fn redact_vector_credentials_does_not_parse_a_string_inside_a_dict() {
    let params = json(r#"{"litellm_embedding_config": "{\"api_key\": \"secret\"}"}"#);
    assert_eq!(
        redact_sensitive_litellm_params(&params, 0, ApiLimits::default()),
        params
    );
}

#[test]
fn serialized_vector_params_redacts_and_reserializes_with_python_spacing() {
    assert_eq!(
        redacted_text(r#"{"api_key":"secret","region":"us"}"#, 0),
        Json::Text(r#"{"api_key": "REDACTED_BY_LITELM", "region": "us"}"#.to_owned())
    );
}

#[test]
fn serialized_vector_params_marks_invalid_json() {
    assert_eq!(
        redacted_text("bad", 0),
        Json::Text(REDACTED_BY_LITELM.to_owned())
    );
    assert_eq!(
        redacted_text("", 0),
        Json::Text(REDACTED_BY_LITELM.to_owned())
    );
}

#[test]
fn serialized_vector_params_keeps_parsed_scalars() {
    for (source, expected) in [
        ("null", "null"),
        ("7", "7"),
        ("true", "true"),
        (r#""text""#, r#""REDACTED_BY_LITELM""#),
    ] {
        assert_eq!(
            redacted_text(source, 0),
            Json::Text(expected.to_owned()),
            "for {source}"
        );
    }
}

#[test]
fn serialized_vector_params_consumes_one_depth_per_parse() {
    let encoded = r#""{\"api_key\": \"secret\"}""#;
    assert_eq!(
        redacted_text(encoded, 0),
        Json::Text(r#""{\"api_key\": \"REDACTED_BY_LITELM\"}""#.to_owned())
    );
}

#[test]
fn serialized_vector_params_consumes_the_depth_limit_before_parsing() {
    assert_eq!(
        redacted_text(r#"{"api_key":"secret"}"#, 9),
        Json::Text(r#""REDACTED_BY_LITELM""#.to_owned())
    );
    assert_eq!(
        redacted_text(r#"{"api_key":"secret"}"#, 10),
        Json::Text(REDACTED_BY_LITELM.to_owned())
    );
}

#[test]
fn serialized_vector_params_reserializes_like_json_dumps() {
    assert_eq!(
        redacted_text(
            r#"{"model": "text-embedding", "note": "a\nb\tc", "api_key": "secret", "items": [1, 2.5e2]}"#,
            0
        ),
        Json::Text(
            r#"{"model": "text-embedding", "note": "a\nb\tc", "api_key": "REDACTED_BY_LITELM", "items": [1, 250.0]}"#
                .to_owned()
        )
    );
}

#[test]
fn decode_management_json_parses_stored_text() {
    assert_eq!(
        decode_management_json(b"{\"a\": 1}", ApiLimits::default()).unwrap(),
        json(r#"{"a": 1}"#)
    );
    assert_eq!(
        decode_management_json(b"7", ApiLimits::default()).unwrap(),
        json("7")
    );
}

#[test]
fn decode_management_json_reports_invalid_text_and_the_limit() {
    assert!(matches!(
        decode_management_json(b"{", ApiLimits::default()),
        Err(ManagementJsonError::Syntax(_))
    ));
    assert!(matches!(
        decode_management_json(b"{\"a\":\"\xff\"}", ApiLimits::default()),
        Err(ManagementJsonError::Syntax(_))
    ));
    let limits = ApiLimits {
        max_json_bytes: 4,
        ..ApiLimits::default()
    };
    assert!(matches!(
        decode_management_json(b"{\"a\": 1}", limits),
        Err(ManagementJsonError::Limits(ApiLimitError::JsonTooLarge {
            observed: 8,
            limit: 4
        }))
    ));
}

#[test]
fn redact_store_params_keeps_a_stored_string_a_string() {
    let original = parsed_record(
        r#"{"vector_store_id":"vs_a","litellm_params":"{\"api_key\":\"secret\",\"region\":\"us\"}","team_id":null}"#,
    );
    assert_eq!(
        rendered(&redact_store_params(&original, ApiLimits::default()).unwrap()),
        json(
            r#"{"vector_store_id":"vs_a","litellm_params":"{\"api_key\": \"REDACTED_BY_LITELM\", \"region\": \"us\"}","team_id":null}"#
        )
    );
}

#[test]
fn redact_store_params_redacts_an_object() {
    let original = parsed_record(
        r#"{"vector_store_id":"vs_b","litellm_params":{"api_key":"secret","region":"us"}}"#,
    );
    assert_eq!(
        rendered(&redact_store_params(&original, ApiLimits::default()).unwrap()),
        json(
            r#"{"vector_store_id":"vs_b","litellm_params":{"api_key":"REDACTED_BY_LITELM","region":"us"}}"#
        )
    );
}

#[test]
fn redact_store_params_leaves_absent_params_absent() {
    let original = parsed_record(r#"{"vector_store_id":"vs_a"}"#);
    assert_eq!(
        redact_store_params(&original, ApiLimits::default()).unwrap(),
        original
    );
}

fn stored_row(id: &str) -> ManagedVectorStoresTable {
    ManagedVectorStoresTable {
        vector_store_id: id.to_owned(),
        custom_llm_provider: "openai".to_owned(),
        vector_store_name: Some("Store".to_owned()),
        vector_store_description: None,
        vector_store_metadata: Some(BTreeMap::from([(
            "env".to_owned(),
            JsonData::String("prod".to_owned()),
        )])),
        created_at: None,
        updated_at: None,
        litellm_credential_name: Some("cred".to_owned()),
        litellm_params: Some(BTreeMap::from([
            ("api_key".to_owned(), JsonData::String("secret".to_owned())),
            ("region".to_owned(), JsonData::String("us".to_owned())),
        ])),
        team_id: None,
        user_id: Some("u1".to_owned()),
        is_config: false,
    }
}

#[test]
fn row_to_vector_store_projects_every_column() {
    let projected = row_to_vector_store(stored_row("vs_a")).unwrap();
    assert!(projected.is_config.is_none());
    assert!(!projected.extra.contains_key("is_config"));
    assert_eq!(
        rendered(&projected),
        json(
            r#"{"vector_store_id":"vs_a","custom_llm_provider":"openai","vector_store_name":"Store","vector_store_description":null,"vector_store_metadata":{"env":"prod"},"created_at":null,"updated_at":null,"litellm_credential_name":"cred","litellm_params":{"api_key":"secret","region":"us"},"team_id":null,"user_id":"u1"}"#
        )
    );
}

struct Rows {
    row: Option<ManagedVectorStoresTable>,
    asked: Mutex<Vec<String>>,
}

impl Rows {
    fn with_row(row: Option<ManagedVectorStoresTable>) -> Self {
        Self {
            row,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl VectorStoreRows for Rows {
    fn find_unique(
        &self,
        vector_store_id: &str,
    ) -> impl Future<Output = Result<Option<ManagedVectorStoresTable>, TableError>> + Send {
        self.asked.lock().unwrap().push(vector_store_id.to_owned());
        ready(Ok(self.row.clone()))
    }
}

#[derive(Default)]
struct World {
    database: bool,
    user_teams: Vec<String>,
    touched: AtomicBool,
}

impl World {
    fn touched(&self) -> bool {
        self.touched.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ObjectPermissionLookup for World {
    fn has_database(&self) -> bool {
        self.touched.store(true, Ordering::SeqCst);
        self.database
    }

    async fn load_permission(
        &self,
        _object_permission_id: &str,
    ) -> Result<Option<LiteLLMObjectPermissionTable>, PermissionError> {
        self.touched.store(true, Ordering::SeqCst);
        Ok(None)
    }
}

#[async_trait]
impl TeamLoader for World {
    async fn load_team(
        &self,
        request: TeamRequest<'_>,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        self.touched.store(true, Ordering::SeqCst);
        Err(TeamLookupError::TeamNotFound {
            team_id: request.team_id.to_owned(),
        })
    }
}

#[async_trait]
impl UiSessionTeams for World {
    fn has_database(&self) -> bool {
        self.touched.store(true, Ordering::SeqCst);
        self.database
    }

    async fn user_teams(
        &self,
        _user_id: &str,
        _fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        self.touched.store(true, Ordering::SeqCst);
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

fn caller(fields: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let mut token = BTreeMap::new();
    for (name, value) in fields {
        token.insert((*name).to_owned(), value.clone());
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(token)).unwrap()
}

#[tokio::test]
async fn check_vector_store_access_forwards_the_record_and_identity() {
    let mut record = row_to_vector_store(stored_row("vs_a")).unwrap();
    record.team_id = Some("t1".to_owned());
    let world = World::default();
    let access = policy(&world);
    let member = caller(&[("team_id", JsonData::String("t1".to_owned()))]);
    let outsider = caller(&[("team_id", JsonData::String("mine".to_owned()))]);
    assert!(
        check_vector_store_access(&access, &record, &member)
            .await
            .unwrap()
    );
    assert!(
        !check_vector_store_access(&access, &record, &outsider)
            .await
            .unwrap()
    );
    assert!(!world.touched());
}

#[tokio::test]
async fn fetch_store_before_access_reports_a_missing_row() {
    let rows = Rows::with_row(None);
    let world = World::default();
    let error = fetch_store_before_access(&rows, &policy(&world), "vs_a", &caller(&[]))
        .await
        .unwrap_err();
    assert_eq!(rows.asked(), vec!["vs_a".to_owned()]);
    assert!(!world.touched());
    let (status, detail) = error.http().unwrap();
    assert_eq!(status, 404);
    assert_eq!(
        detail,
        VectorManagementDetail::Text("Vector store with ID vs_a not found".to_owned())
    );
}

#[tokio::test]
async fn fetch_store_before_access_reports_a_denied_row() {
    let mut row = stored_row("vs_a");
    row.team_id = Some("other".to_owned());
    let rows = Rows::with_row(Some(row));
    let world = World::default();
    let denied = caller(&[("team_id", JsonData::String("mine".to_owned()))]);
    let error = fetch_store_before_access(&rows, &policy(&world), "vs_a", &denied)
        .await
        .unwrap_err();
    let (status, detail) = error.http().unwrap();
    assert_eq!(status, 403);
    assert_eq!(
        detail,
        VectorManagementDetail::Text(ACCESS_DENIED.to_owned())
    );
}

#[tokio::test]
async fn fetch_store_before_access_returns_the_whole_row_for_an_allowed_call() {
    let rows = Rows::with_row(Some(stored_row("vs_a")));
    let world = World::default();
    let store = fetch_store_before_access(&rows, &policy(&world), "vs_a", &caller(&[]))
        .await
        .unwrap();
    assert_eq!(rows.asked(), vec!["vs_a".to_owned()]);
    assert_eq!(store, row_to_vector_store(stored_row("vs_a")).unwrap());
    assert!(!world.touched());
}

#[tokio::test]
async fn fetch_store_before_access_propagates_a_team_lookup_failure() {
    let mut row = stored_row("vs_a");
    row.team_id = Some("t1".to_owned());
    let rows = Rows::with_row(Some(row));
    let world = World {
        database: true,
        user_teams: vec!["t1".to_owned()],
        touched: AtomicBool::new(false),
    };
    let session = caller(&[
        (
            "team_id",
            JsonData::String(UI_SESSION_TOKEN_TEAM_ID.to_owned()),
        ),
        ("user_id", JsonData::String("u1".to_owned())),
    ]);
    let error = fetch_store_before_access(&rows, &policy(&world), "vs_a", &session)
        .await
        .unwrap_err();
    let (status, detail) = error.http().unwrap();
    assert_eq!(status, 404);
    assert_eq!(
        detail,
        VectorManagementDetail::Error(ErrorDetail {
            error: "Team doesn't exist in db. Team=t1. Create team via `/team/new` call."
                .to_owned()
        })
    );
}

#[test]
fn replays_corpus() {
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/fuzz/corpus/management_json");
    let outcomes: Vec<bool> = fs::read_dir(directory)
        .expect("the corpus exists")
        .map(|entry| {
            let bytes = fs::read(entry.expect("the entry lists").path()).expect("the seed reads");
            let Ok(value) = decode_management_json(&bytes, ApiLimits::default()) else {
                return false;
            };
            let redacted = redact_sensitive_litellm_params(&value, 0, ApiLimits::default());
            let reparsed: Json = serde_json::from_str(&dumps(&redacted))
                .expect("a redacted value writes back as JSON");
            assert_eq!(reparsed, redacted);
            true
        })
        .collect();
    assert!(outcomes.len() >= 8);
    assert!(outcomes.contains(&true));
    assert!(outcomes.contains(&false));
}
