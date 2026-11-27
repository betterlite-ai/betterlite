#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::mcp_management_mode::ManagementMode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::managed_resource_rows::wire::RowFlag;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_mcp::registry::resolution::{
    DbLookup, LookupFuture, ResolveError, ServerAccessCheck,
};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::registry::table::ServerTables as _;
use betterlite_mcp::server_record::MCPServer;
use rstest::rstest;

use super::{
    AccessibleServerSources, ManagementAccessError, authorize_and_fetch_mcp_server,
    is_restricted_virtual_key_request, resolve_accessible_mcp_servers,
};

const NO_ROUTES: &str = r#"{"token":"key-1"}"#;
const NULL_ROUTES: &str = r#"{"token":"key-1","allowed_routes":null}"#;
const EMPTY_ROUTES: &str = r#"{"token":"key-1","allowed_routes":[]}"#;
const RESTRICTED_ROUTES: &str = r#"{"token":"key-1","allowed_routes":["/v1/mcp/server"]}"#;
const ADMIN: &str = r#"{"user_role":"proxy_admin"}"#;
const ADMIN_VIEW: &str = r#"{"user_role":"proxy_admin_viewer"}"#;
const NON_ADMIN: &str = r#"{"user_id":"user-1"}"#;
const SERVER_ONE: &str = r#"{"server_id":"test-srv-1","name":"Server One","transport":"http"}"#;
const SERVER_TWO_FIRST: &str = r#"{"server_id":"test-srv-2","name":"Server Two","transport":"http","url":"https://first.example"}"#;
const SERVER_TWO_LATER: &str = r#"{"server_id":"test-srv-2","name":"Server Two","transport":"http","url":"https://later.example"}"#;
const SERVER_THREE: &str = r#"{"server_id":"test-srv-3","name":"Server Three","transport":"http"}"#;

fn key(fields: &str) -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).unwrap()
}

const fn registry_settings() -> RegistrySettings {
    RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: true,
        internal_ip_ranges: None,
    }
}

fn record(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn registry(config: &[&str], db: &[&str]) -> ServerRegistry {
    let config_servers = config.iter().copied().map(record).collect();
    let db_servers = db.iter().copied().map(record).collect();
    ServerRegistry::new(registry_settings(), config_servers, db_servers)
}

fn server_table(json: &str) -> McpServerTable {
    registry(&[json], &[]).all_server_tables().remove(0)
}

fn database_table(json: &str) -> McpServerTable {
    registry(&[], &[json]).all_server_tables().remove(0)
}

type DbAnswer = Result<Option<McpServerTable>, ResolveError>;

struct FakeDb {
    answer: Mutex<Option<DbAnswer>>,
    asked: Mutex<Vec<String>>,
}

impl FakeDb {
    const fn answering(table: Option<McpServerTable>) -> Self {
        Self {
            answer: Mutex::new(Some(Ok(table))),
            asked: Mutex::new(Vec::new()),
        }
    }

    const fn failing(error: ResolveError) -> Self {
        Self {
            answer: Mutex::new(Some(Err(error))),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked_ids(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl DbLookup for FakeDb {
    fn db_table<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<McpServerTable>> {
        self.asked.lock().unwrap().push(server_id.to_owned());
        let answer = self.answer.lock().unwrap().take().unwrap_or(Ok(None));
        Box::pin(ready(answer))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the access check failed")]
struct AccessFailure;

struct FakeAccess {
    allows: bool,
    fails: bool,
    asked: Mutex<Vec<String>>,
}

impl FakeAccess {
    const fn allowing(allows: bool) -> Self {
        Self {
            allows,
            fails: false,
            asked: Mutex::new(Vec::new()),
        }
    }

    const fn failing() -> Self {
        Self {
            allows: false,
            fails: true,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked_ids(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ServerAccessCheck for FakeAccess {
    type Error = AccessFailure;

    fn can_access(
        &self,
        _auth: &UserAPIKeyAuth,
        server_id: &str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        self.asked.lock().unwrap().push(server_id.to_owned());
        let answer = if self.fails {
            Err(AccessFailure)
        } else {
            Ok(self.allows)
        };
        ready(answer)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("source failure {number}")]
struct SourceFailure {
    number: u8,
}

struct FakeSources {
    contexts: Result<Vec<UserAPIKeyAuth>, SourceFailure>,
    tables: Mutex<VecDeque<Result<Vec<McpServerTable>, SourceFailure>>>,
    context_calls: AtomicUsize,
    table_calls: AtomicUsize,
}

impl FakeSources {
    fn answering(contexts: Vec<UserAPIKeyAuth>, tables: Vec<Vec<McpServerTable>>) -> Self {
        Self {
            contexts: Ok(contexts),
            tables: Mutex::new(tables.into_iter().map(Ok).collect()),
            context_calls: AtomicUsize::new(0),
            table_calls: AtomicUsize::new(0),
        }
    }

    const fn failing_contexts(number: u8) -> Self {
        Self {
            contexts: Err(SourceFailure { number }),
            tables: Mutex::new(VecDeque::new()),
            context_calls: AtomicUsize::new(0),
            table_calls: AtomicUsize::new(0),
        }
    }

    fn failing_tables(contexts: Vec<UserAPIKeyAuth>, number: u8) -> Self {
        Self {
            contexts: Ok(contexts),
            tables: Mutex::new(VecDeque::from([Err(SourceFailure { number })])),
            context_calls: AtomicUsize::new(0),
            table_calls: AtomicUsize::new(0),
        }
    }

    fn contexts_asked(&self) -> usize {
        self.context_calls.load(Ordering::SeqCst)
    }

    fn tables_asked(&self) -> usize {
        self.table_calls.load(Ordering::SeqCst)
    }
}

impl AccessibleServerSources for FakeSources {
    type Error = SourceFailure;

    fn effective_auth_contexts(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, Self::Error>> + Send {
        self.context_calls.fetch_add(1, Ordering::SeqCst);
        ready(self.contexts.clone())
    }

    fn allowed_server_tables(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<McpServerTable>, Self::Error>> + Send {
        self.table_calls.fetch_add(1, Ordering::SeqCst);
        let answer = self
            .tables
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()));
        ready(answer)
    }
}

fn not_found_detail(error: ManagementAccessError<AccessFailure>) -> BTreeMap<String, String> {
    match error {
        ManagementAccessError::NotFound(detail) => detail,
        ManagementAccessError::Forbidden(_)
        | ManagementAccessError::Lookup(_)
        | ManagementAccessError::Access(_) => panic!("expected a not-found denial"),
    }
}

fn forbidden_detail(error: ManagementAccessError<AccessFailure>) -> BTreeMap<String, String> {
    match error {
        ManagementAccessError::Forbidden(detail) => detail,
        ManagementAccessError::NotFound(_)
        | ManagementAccessError::Lookup(_)
        | ManagementAccessError::Access(_) => panic!("expected a forbidden denial"),
    }
}

#[rstest]
#[case::without_a_list(NO_ROUTES, false)]
#[case::null_routes(NULL_ROUTES, false)]
#[case::empty_list(EMPTY_ROUTES, false)]
#[case::non_empty_list(RESTRICTED_ROUTES, true)]
fn restricted_key_detects_only_a_non_empty_allowed_routes(
    #[case] fields: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_restricted_virtual_key_request(&key(fields)), expected);
}

#[tokio::test]
async fn authorize_admin_reaches_a_database_server() {
    let loaded = registry(&[], &[]);
    let db = FakeDb::answering(Some(database_table(SERVER_ONE)));
    let access = FakeAccess::allowing(false);
    let found = authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN), "test-srv-1")
        .await
        .unwrap();
    assert_eq!(found.server_id, "test-srv-1");
    assert_eq!(found.is_config, RowFlag(false));
    assert_eq!(db.asked_ids(), vec!["test-srv-1".to_owned()]);
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_admin_reaches_a_config_only_server() {
    let loaded = registry(&[SERVER_ONE], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(false);
    let found = authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN), "test-srv-1")
        .await
        .unwrap();
    assert_eq!(found.server_id, "test-srv-1");
    assert_eq!(found.is_config, RowFlag(true));
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_view_only_admin_reaches_a_restricted_server() {
    let loaded = registry(&[SERVER_ONE], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(false);
    let found =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN_VIEW), "test-srv-1")
            .await
            .unwrap();
    assert_eq!(found.server_id, "test-srv-1");
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_admin_gets_the_not_found_text_for_an_unknown_id() {
    let loaded = registry(&[], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(true);
    let error = authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN), "test-ghost")
        .await
        .unwrap_err();
    let detail = not_found_detail(error);
    assert_eq!(
        detail.get("error").map(String::as_str),
        Some("MCP Server test-ghost not found")
    );
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_view_only_admin_gets_the_not_found_text_for_an_unknown_id() {
    let loaded = registry(&[], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(true);
    let error =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN_VIEW), "test-ghost")
            .await
            .unwrap_err();
    let detail = not_found_detail(error);
    assert_eq!(
        detail.get("error").map(String::as_str),
        Some("MCP Server test-ghost not found")
    );
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_non_admin_reaches_an_allowed_server() {
    let loaded = registry(&[SERVER_ONE], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(true);
    let found =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(NON_ADMIN), "test-srv-1")
            .await
            .unwrap();
    assert_eq!(found.server_id, "test-srv-1");
    assert_eq!(access.asked_ids(), vec!["test-srv-1".to_owned()]);
}

#[tokio::test]
async fn authorize_non_admin_denied_gets_the_forbidden_text() {
    let loaded = registry(&[SERVER_ONE], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(false);
    let error =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(NON_ADMIN), "test-srv-1")
            .await
            .unwrap_err();
    let detail = forbidden_detail(error);
    assert_eq!(
        detail.get("error").map(String::as_str),
        Some(
            "User does not have permission to access mcp server with id test-srv-1. You can only manage mcp servers that you have access to."
        )
    );
    assert_eq!(access.asked_ids(), vec!["test-srv-1".to_owned()]);
}

#[tokio::test]
async fn authorize_non_admin_unknown_id_is_forbidden_not_missing() {
    let loaded = registry(&[], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::allowing(true);
    let error =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(NON_ADMIN), "test-ghost")
            .await
            .unwrap_err();
    let detail = forbidden_detail(error);
    assert_eq!(
        detail.get("error").map(String::as_str),
        Some(
            "User does not have permission to access mcp server with id test-ghost. You can only manage mcp servers that you have access to."
        )
    );
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_lookup_error_is_a_lookup_failure() {
    let loaded = registry(&[], &[]);
    let db = FakeDb::failing(ResolveError::from(McpTableError::ServerNotFound {
        server_id: "test-srv-1".to_owned(),
    }));
    let access = FakeAccess::allowing(true);
    let error = authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(ADMIN), "test-srv-1")
        .await
        .unwrap_err();
    assert!(matches!(error, ManagementAccessError::Lookup(_)));
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn authorize_access_error_is_not_a_denial() {
    let loaded = registry(&[SERVER_ONE], &[]);
    let db = FakeDb::answering(None);
    let access = FakeAccess::failing();
    let error =
        authorize_and_fetch_mcp_server(&loaded, &db, &access, &key(NON_ADMIN), "test-srv-1")
            .await
            .unwrap_err();
    assert!(matches!(error, ManagementAccessError::Access(_)));
    assert_eq!(access.asked_ids(), vec!["test-srv-1".to_owned()]);
}

#[tokio::test]
async fn accessible_view_all_unrestricted_answers_every_registry_table() {
    let loaded = registry(&[SERVER_ONE, SERVER_TWO_FIRST], &[SERVER_THREE]);
    let sources = FakeSources::answering(Vec::new(), Vec::new());
    let tables =
        resolve_accessible_mcp_servers(&loaded, &sources, &key(NO_ROUTES), ManagementMode::ViewAll)
            .await
            .unwrap();
    let ids: Vec<&str> = tables
        .iter()
        .map(|table| table.server_id.as_str())
        .collect();
    assert_eq!(ids, ["test-srv-1", "test-srv-2", "test-srv-3"]);
    assert_eq!(sources.contexts_asked(), 0);
    assert_eq!(sources.tables_asked(), 0);
}

#[rstest]
#[case::view_all_with_a_restricted_key(ManagementMode::ViewAll, RESTRICTED_ROUTES)]
#[case::restricted_mode_with_an_unrestricted_key(ManagementMode::Restricted, NO_ROUTES)]
#[case::restricted_mode_with_a_restricted_key(ManagementMode::Restricted, RESTRICTED_ROUTES)]
#[tokio::test]
async fn accessible_walk_runs_for_view_all_restricted_and_restricted_mode(
    #[case] mode: ManagementMode,
    #[case] fields: &'static str,
) {
    let loaded = registry(&[SERVER_ONE], &[]);
    let sources =
        FakeSources::answering(vec![key(NO_ROUTES)], vec![vec![server_table(SERVER_ONE)]]);
    let tables = resolve_accessible_mcp_servers(&loaded, &sources, &key(fields), mode)
        .await
        .unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].server_id, "test-srv-1");
    assert_eq!(sources.contexts_asked(), 1);
    assert_eq!(sources.tables_asked(), 1);
}

#[tokio::test]
async fn accessible_walk_keeps_the_first_table_seen() {
    let loaded = registry(&[], &[]);
    let sources = FakeSources::answering(
        vec![key(NO_ROUTES), key(NON_ADMIN)],
        vec![
            vec![server_table(SERVER_ONE), server_table(SERVER_TWO_FIRST)],
            vec![server_table(SERVER_TWO_LATER), server_table(SERVER_THREE)],
        ],
    );
    let tables = resolve_accessible_mcp_servers(
        &loaded,
        &sources,
        &key(NO_ROUTES),
        ManagementMode::Restricted,
    )
    .await
    .unwrap();
    let ids: Vec<&str> = tables
        .iter()
        .map(|table| table.server_id.as_str())
        .collect();
    assert_eq!(ids, ["test-srv-1", "test-srv-2", "test-srv-3"]);
    assert_eq!(tables[1].url.as_deref(), Some("https://first.example"));
    assert_eq!(sources.contexts_asked(), 1);
    assert_eq!(sources.tables_asked(), 2);
}

#[tokio::test]
async fn accessible_context_error_comes_back_unchanged() {
    let loaded = registry(&[], &[]);
    let sources = FakeSources::failing_contexts(1);
    let error = resolve_accessible_mcp_servers(
        &loaded,
        &sources,
        &key(NO_ROUTES),
        ManagementMode::Restricted,
    )
    .await
    .unwrap_err();
    assert_eq!(error, SourceFailure { number: 1 });
    assert_eq!(sources.tables_asked(), 0);
}

#[tokio::test]
async fn accessible_table_error_comes_back_unchanged() {
    let loaded = registry(&[], &[]);
    let sources = FakeSources::failing_tables(vec![key(NO_ROUTES)], 2);
    let error = resolve_accessible_mcp_servers(
        &loaded,
        &sources,
        &key(NO_ROUTES),
        ManagementMode::Restricted,
    )
    .await
    .unwrap_err();
    assert_eq!(error, SourceFailure { number: 2 });
    assert_eq!(sources.contexts_asked(), 1);
    assert_eq!(sources.tables_asked(), 1);
}
