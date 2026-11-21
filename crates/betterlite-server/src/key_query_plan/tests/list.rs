use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};

use rust_decimal::Decimal;
use tracing::subscriber::set_default;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::OptionalPayload;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::key_query_plan::KeyQueryPlanError;
use crate::key_query_plan::list::{list_keys, list_keys_with};
use crate::key_query_plan::plan::{KeyQueryPlan, KeyTable};
use crate::key_query_plan::response::{KeyListEntry, KeyRow, entry_for_row};
use crate::key_query_plan::sort::SortError;
use crate::key_query_plan::source::KeyListSource;

use super::query;

fn row(document: &str) -> KeyRow {
    let value: JsonData = serde_json::from_str(document).unwrap();
    let JsonData::Object(fields) = value else {
        panic!("a key row is a JSON object");
    };
    fields
}

fn entry_fields(entry: &KeyListEntry) -> BTreeMap<String, JsonData> {
    let document = serde_json::to_string(entry).unwrap();
    let value: JsonData = serde_json::from_str(&document).unwrap();
    let JsonData::Object(fields) = value else {
        panic!("a full key entry serializes to an object");
    };
    fields
}

fn user_table(user_id: &str, email: Option<&str>, alias: Option<&str>) -> LiteLLMUserTable {
    LiteLLMUserTable {
        user_id: user_id.to_owned(),
        user_alias: alias.map(str::to_owned),
        team_id: None,
        sso_user_id: None,
        organization_id: None,
        object_permission_id: None,
        password: None,
        password_reset_required: None,
        last_breach_check_at: None,
        teams: Vec::new(),
        user_role: None,
        max_budget: None,
        spend: Decimal::ZERO,
        user_email: email.map(str::to_owned),
        models: Vec::new(),
        metadata: None,
        max_parallel_requests: None,
        tpm_limit: None,
        rpm_limit: None,
        budget_duration: None,
        budget_reset_at: None,
        allowed_cache_controls: Vec::new(),
        policies: Vec::new(),
        model_spend: OptionalPayload::default(),
        model_max_budget: OptionalPayload::default(),
        created_at: None,
        updated_at: None,
        organization_memberships: None,
        object_permission: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    Page,
    Count,
    Users,
    Attach,
}

struct FakeSource {
    page: Vec<KeyRow>,
    total: i64,
    users: Vec<LiteLLMUserTable>,
    failure: Option<Failure>,
    calls: Mutex<Vec<String>>,
    attach_calls: Mutex<usize>,
}

impl FakeSource {
    fn record(&self, name: &str) {
        self.calls.lock().unwrap().push(name.to_owned());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn attach_calls(&self) -> usize {
        *self.attach_calls.lock().unwrap()
    }

    fn answer<T>(&self, stage: Failure, value: T) -> Result<T, KeyQueryPlanError> {
        if self.failure == Some(stage) {
            return Err(KeyQueryPlanError::MissingToken);
        }
        Ok(value)
    }
}

impl KeyListSource for FakeSource {
    fn fetch_page(
        &self,
        _plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<Vec<KeyRow>, KeyQueryPlanError>> + Send {
        self.record("page");
        async move { self.answer(Failure::Page, self.page.clone()) }
    }

    fn count(
        &self,
        _plan: &KeyQueryPlan,
    ) -> impl Future<Output = Result<i64, KeyQueryPlanError>> + Send {
        self.record("count");
        async move { self.answer(Failure::Count, self.total) }
    }

    fn find_users(
        &self,
        _ids: Vec<String>,
    ) -> impl Future<Output = Result<Vec<LiteLLMUserTable>, KeyQueryPlanError>> + Send {
        self.record("users");
        async move {
            let users = self.users.clone();
            self.answer(Failure::Users, users)
        }
    }

    fn attach_object_permission(
        &self,
        _row: &mut KeyRow,
    ) -> impl Future<Output = Result<(), KeyQueryPlanError>> + Send {
        self.record("attach");
        let mut count = self.attach_calls.lock().unwrap();
        *count = count.saturating_add(1);
        drop(count);
        async move { self.answer(Failure::Attach, ()) }
    }
}

#[tokio::test]
async fn user_lookup_without_database_errors() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let ids = vec![String::from("alice")];
    let error = KeyListSource::find_users(&client, ids).await.unwrap_err();
    assert!(matches!(
        error,
        KeyQueryPlanError::Database(DbError::NoDbConnected)
    ));
}

#[tokio::test]
async fn empty_page_zero_pages() {
    let source = FakeSource {
        page: Vec::new(),
        total: 0,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut request = query(1, 10);
    request.return_full_object = true;
    let response = list_keys_with(&source, &request).await.unwrap();
    assert!(response.keys.is_empty());
    assert_eq!(response.total_count, Some(0));
    assert_eq!(response.current_page, Some(1));
    assert_eq!(response.total_pages, Some(0));
    assert_eq!(source.attach_calls(), 0);
}

#[test]
fn dump_fallback() {
    // the mapping a pydantic v1 dict() dump would give: organization_id and no model attributes
    let mapping = row(
        r#"{"token":"hash-1","key_alias":"a","organization_id":"org-1","litellm_budget_table":{"budget_id":"b1"}}"#,
    );
    let entry = entry_for_row(mapping, KeyTable::Live, true).unwrap();
    let fields = entry_fields(&entry);
    assert_eq!(
        fields.get("org_id"),
        Some(&JsonData::String(String::from("org-1")))
    );
    assert!(!fields.contains_key("organization_id"));
    // the list loop inserts no derived status; the stored fields are what the caller sees
    assert!(!fields.contains_key("status"));
    assert!(matches!(
        fields.get("litellm_budget_table"),
        Some(JsonData::Object(_))
    ));
}

#[test]
fn full_object_null_fields_take_defaults() {
    // the flattened auth view rejects a null at its custom decimal visitors, and LiteLLM reads a null as the field's default
    let live = row(
        r#"{"token":"k1","max_budget":null,"spend":null,"litellm_budget_table":null,"object_permission":null}"#,
    );
    let entry = entry_for_row(live, KeyTable::Live, true).unwrap();
    let fields = entry_fields(&entry);
    assert_eq!(fields.get("max_budget"), Some(&JsonData::Null));
    assert!(
        matches!(fields.get("spend"), Some(JsonData::Number(_))),
        "a null spend reads as the zero default"
    );
    assert_eq!(fields.get("litellm_budget_table"), Some(&JsonData::Null));
}

#[test]
fn full_live_vs_archived_rows() {
    let live = row(r#"{"token":"live-1","key_alias":"a"}"#);
    let live_entry = entry_for_row(live, KeyTable::Live, true).unwrap();
    assert!(matches!(live_entry, KeyListEntry::Auth(_)));

    let archived = archived_row();
    let archived_entry = entry_for_row(archived.clone(), KeyTable::Deleted, true).unwrap();
    let fields = entry_fields(&archived_entry);
    assert_eq!(
        fields.get("deleted_by"),
        Some(&JsonData::String(String::from("admin")))
    );
    assert_eq!(
        fields.get("deleted_by_api_key"),
        Some(&JsonData::String(String::from("op-hash")))
    );
    assert_eq!(
        fields.get("token"),
        Some(&JsonData::String(String::from("del-1")))
    );
    assert!(matches!(
        entry_for_row(archived, KeyTable::Deleted, false).unwrap(),
        KeyListEntry::Token(token) if token == "del-1"
    ));
}

fn archived_row() -> KeyRow {
    row(
        r#"{"id":"d1","token":"del-1","deleted_at":"2025-01-01T00:00:00","deleted_by":"admin","deleted_by_api_key":"op-hash","litellm_changed_by":null}"#,
    )
}

#[derive(Clone)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn page_then_count_not_snapshot() {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let guard = set_default(subscriber);
    let source = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#)],
        total: 7,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let response = list_keys_with(&source, &query(2, 2)).await.unwrap();
    assert_eq!(
        source.calls(),
        vec![
            String::from("page"),
            String::from("count"),
            String::from("attach")
        ]
    );
    // the page holds one row while the count already saw seven, so no snapshot spans both reads
    assert_eq!(response.keys.len(), 1);
    assert_eq!(response.total_count, Some(7));
    assert_eq!(response.total_pages, Some(4));
    assert_eq!(response.current_page, Some(2));
    drop(guard);
    let bytes = captured.0.lock().unwrap().clone();
    let output = String::from_utf8(bytes).unwrap();
    assert!(output.contains("Filter conditions"));
    assert!(output.contains("skip=2"));
    assert!(output.contains("take=2"));
    assert!(output.contains("Fetched keys"));
    assert!(output.contains("Total count of keys"));
}

#[tokio::test]
async fn expand_owner_and_creator() {
    let source = FakeSource {
        page: vec![row(
            r#"{"token":"k1","user_id":"alice","created_by":"bob"}"#,
        )],
        total: 1,
        users: vec![
            user_table("alice", Some("alice@example.com"), Some("Alice")),
            user_table("bob", Some("bob@example.com"), None),
        ],
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut request = query(1, 10);
    request.expand = vec![String::from("user")];
    let response = list_keys_with(&source, &request).await.unwrap();
    let fields = entry_fields(&response.keys[0]);
    let Some(JsonData::Object(user)) = fields.get("user") else {
        panic!("the owner expansion is an object");
    };
    assert_eq!(
        user.get("user_id"),
        Some(&JsonData::String(String::from("alice")))
    );
    let Some(JsonData::Object(creator)) = fields.get("created_by_user") else {
        panic!("the creator expansion is an object");
    };
    assert_eq!(
        creator.get("user_email"),
        Some(&JsonData::String(String::from("bob@example.com")))
    );
    assert_eq!(creator.get("user_alias"), Some(&JsonData::Null));
    // expand=user alone returns full objects
    assert!(matches!(response.keys[0], KeyListEntry::Auth(_)));
}

#[tokio::test]
async fn expand_missing_users_and_unknown_names() {
    let source = FakeSource {
        page: vec![row(
            r#"{"token":"k1","user_id":"ghost","created_by":"phantom"}"#,
        )],
        total: 1,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut request = query(1, 10);
    request.expand = vec![String::from("user")];
    let response = list_keys_with(&source, &request).await.unwrap();
    let fields = entry_fields(&response.keys[0]);
    // the expansions were not found, so the shared auth shape carries them as null
    assert_eq!(fields.get("user"), Some(&JsonData::Null));
    assert_eq!(fields.get("created_by_user"), Some(&JsonData::Null));
    assert!(source.calls().contains(&String::from("users")));

    let unknown = FakeSource {
        page: vec![row(r#"{"token":"k1","user_id":"alice"}"#)],
        total: 1,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut unknown_request = query(1, 10);
    unknown_request.expand = vec![String::from("bogus")];
    let unknown_response = list_keys_with(&unknown, &unknown_request).await.unwrap();
    assert!(!unknown.calls().contains(&String::from("users")));
    assert!(matches!(unknown_response.keys[0], KeyListEntry::Token(_)));
}

#[tokio::test]
async fn empty_identifiers_skip_user_lookup() {
    let source = FakeSource {
        page: vec![row(r#"{"token":"k1","user_id":"","created_by":""}"#)],
        total: 1,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut request = query(1, 10);
    request.expand = vec![String::from("user")];
    let response = list_keys_with(&source, &request).await.unwrap();
    // an empty identifier is no user to look up, so no query runs
    assert!(!source.calls().contains(&String::from("users")));
    let fields = entry_fields(&response.keys[0]);
    assert_eq!(fields.get("user"), Some(&JsonData::Null));
    assert_eq!(fields.get("created_by_user"), Some(&JsonData::Null));
}

#[tokio::test]
async fn token_only_still_attaches_permission() {
    let source = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#), row(r#"{"token":"k2"}"#)],
        total: 2,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let request = query(1, 10);
    let response = list_keys_with(&source, &request).await.unwrap();
    assert_eq!(source.attach_calls(), 2);
    assert!(
        response
            .keys
            .iter()
            .all(|entry| matches!(entry, KeyListEntry::Token(_)))
    );

    let failing = FakeSource {
        page: vec![row(r#"{"token":"k1"}"#)],
        total: 1,
        users: Vec::new(),
        failure: Some(Failure::Attach),
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let failure = list_keys_with(&failing, &request).await.unwrap_err();
    assert!(matches!(failure, KeyQueryPlanError::MissingToken));
}

#[tokio::test]
async fn query_failures_propagate() {
    let mut invalid_sort = query(1, 10);
    invalid_sort.sort_by = Some(String::from("created"));
    let untouched = FakeSource {
        page: Vec::new(),
        total: 0,
        users: Vec::new(),
        failure: None,
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let sort_error = list_keys_with(&untouched, &invalid_sort).await.unwrap_err();
    assert!(matches!(
        sort_error,
        KeyQueryPlanError::Sort(SortError::Column { .. })
    ));
    assert!(
        untouched.calls().is_empty(),
        "no query runs without a valid sort"
    );

    failed_stage(Failure::Page, 0, &["page"]).await;
    failed_stage(Failure::Count, 3, &["page", "count"]).await;

    let users_failure = FakeSource {
        page: vec![row(r#"{"token":"k1","user_id":"alice"}"#)],
        total: 1,
        users: Vec::new(),
        failure: Some(Failure::Users),
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let mut expanded = query(1, 10);
    expanded.expand = vec![String::from("user")];
    let users_error = list_keys_with(&users_failure, &expanded).await.unwrap_err();
    assert!(matches!(users_error, KeyQueryPlanError::MissingToken));
    assert!(users_failure.calls().contains(&String::from("users")));

    let no_client = list_keys(None, &query(1, 10)).await.unwrap_err();
    assert!(matches!(
        no_client,
        KeyQueryPlanError::Database(DbError::NoDbConnected)
    ));
}

async fn failed_stage(stage: Failure, total: i64, expected: &[&str]) {
    let source = FakeSource {
        page: Vec::new(),
        total,
        users: Vec::new(),
        failure: Some(stage),
        calls: Mutex::new(Vec::new()),
        attach_calls: Mutex::new(0),
    };
    let error = list_keys_with(&source, &query(1, 10)).await.unwrap_err();
    assert!(matches!(error, KeyQueryPlanError::MissingToken));
    assert_eq!(source.calls(), expected);
}
