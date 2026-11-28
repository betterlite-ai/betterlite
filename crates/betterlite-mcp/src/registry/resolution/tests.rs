#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::types::McpTableError;

use super::{
    AuthorizeDetails, AuthorizeError, AuthorizeOptions, DbLookup, LookupFuture, NonAdminMissing,
    ResolutionSource, ResolveError, ResolveTarget, ResolvedServer, ServerAccessCheck, TempLookup,
    authorize_server, resolve_server,
};
use crate::registry::table::ServerTables as _;
use crate::registry::tests::support::{record, registry, registry_with, settings};
use crate::server_build::BuildError;
use crate::server_record::MCPServer;

const BETA: &str = r#"{"server_id":"test-b","name":"Beta","transport":"http"}"#;
const PRIVATE: &str = r#"{"server_id":"test-p","name":"Private Name","transport":"http","available_on_public_internet":false}"#;
const NAMED_PRIVATE: &str = r#"{"server_id":"test-n","name":"test-named","transport":"http","available_on_public_internet":false}"#;
const TEMP: &str = r#"{"server_id":"test-temp","name":"Temp Server","transport":"http"}"#;

type Scripted<Item> = Mutex<Option<Result<Option<Item>, ResolveError>>>;
type FakeTemp = FakeLookup<MCPServer>;
type FakeDb = FakeLookup<McpServerTable>;

struct FakeLookup<Item> {
    script: Scripted<Item>,
    asked: Mutex<Vec<String>>,
}

impl<Item> FakeLookup<Item> {
    const fn answering(answer: Option<Item>) -> Self {
        Self {
            script: Mutex::new(Some(Ok(answer))),
            asked: Mutex::new(Vec::new()),
        }
    }

    const fn failing(error: ResolveError) -> Self {
        Self {
            script: Mutex::new(Some(Err(error))),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked_ids(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl<Item: Send + 'static> FakeLookup<Item> {
    fn answer_for<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<Item>> {
        self.asked.lock().unwrap().push(server_id.to_owned());
        let answer = self.script.lock().unwrap().take().unwrap_or(Ok(None));
        Box::pin(ready(answer))
    }
}

impl TempLookup for FakeTemp {
    fn temp_server<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<MCPServer>> {
        self.answer_for(server_id)
    }
}

impl DbLookup for FakeDb {
    fn db_table<'lookup>(
        &'lookup self,
        server_id: &'lookup str,
    ) -> LookupFuture<'lookup, Option<McpServerTable>> {
        self.answer_for(server_id)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the access check could not run")]
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

fn auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

fn details() -> AuthorizeDetails {
    let not_found = BTreeMap::from([("detail".to_owned(), "Server not found".to_owned())]);
    let forbidden = BTreeMap::from([("detail".to_owned(), "Access denied".to_owned())]);
    AuthorizeDetails {
        not_found,
        forbidden,
    }
}

const fn options(non_admin_missing: NonAdminMissing) -> AuthorizeOptions {
    AuthorizeOptions {
        is_admin_view: false,
        allow_catalog_view: false,
        non_admin_missing,
    }
}

const fn target(server_id: &str) -> ResolveTarget<'_> {
    ResolveTarget {
        server_id,
        id_client_ip: None,
        name_client_ip: None,
        match_name: false,
    }
}

fn table_of(json: &str) -> McpServerTable {
    let mut tables = registry(&[json], &[]).all_server_tables();
    tables.remove(0)
}

fn registry_resolution(json: &str) -> ResolvedServer {
    ResolvedServer {
        table: table_of(json),
        runtime: Some(record(json)),
        source: ResolutionSource::Registry,
    }
}

fn temp_resolution(json: &str) -> ResolvedServer {
    ResolvedServer {
        table: table_of(json),
        runtime: Some(record(json)),
        source: ResolutionSource::Temp,
    }
}

#[tokio::test]
async fn resolution_temp_record_wins() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::answering(Some(record(TEMP)));
    let lookup: &dyn TempLookup = &temp;
    let found = resolve_server(&loaded, target("test-temp"), Some(lookup), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.source, ResolutionSource::Temp);
    assert!(found.runtime.is_some());
    assert_eq!(found.table.server_id, "test-temp");
    assert_eq!(temp.asked_ids(), vec!["test-temp"]);
}

#[tokio::test]
async fn resolution_temp_wins_over_the_database() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::answering(Some(record(TEMP)));
    let db = FakeDb::answering(Some(table_of(BETA)));
    let temp_lookup: &dyn TempLookup = &temp;
    let db_lookup: &dyn DbLookup = &db;
    let found = resolve_server(
        &loaded,
        target("test-temp"),
        Some(temp_lookup),
        Some(db_lookup),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(found.source, ResolutionSource::Temp);
    assert!(db.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_database_row_wins_over_the_registry() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::answering(None);
    let db = FakeDb::answering(Some(table_of(BETA)));
    let temp_lookup: &dyn TempLookup = &temp;
    let db_lookup: &dyn DbLookup = &db;
    let found = resolve_server(
        &loaded,
        target("test-b"),
        Some(temp_lookup),
        Some(db_lookup),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(found.source, ResolutionSource::Db);
    assert!(found.runtime.is_none());
    assert_eq!(found.table.server_id, "test-b");
    assert_eq!(temp.asked_ids(), vec!["test-b"]);
    assert_eq!(db.asked_ids(), vec!["test-b"]);
}

#[tokio::test]
async fn resolution_null_lookups_fall_through_to_the_registry() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::answering(None);
    let db = FakeDb::answering(None);
    let temp_lookup: &dyn TempLookup = &temp;
    let db_lookup: &dyn DbLookup = &db;
    let found = resolve_server(
        &loaded,
        target("test-b"),
        Some(temp_lookup),
        Some(db_lookup),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(found.source, ResolutionSource::Registry);
    assert!(found.runtime.is_some());
}

#[tokio::test]
async fn resolution_without_any_source_is_none() {
    let loaded = registry(&[], &[]);
    assert!(
        resolve_server(&loaded, target("test-b"), None, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn resolution_temp_error_stops_the_chain() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::failing(ResolveError::from(BuildError::PrefixExhausted {
        server_id: "test-b".to_owned(),
        attempts: 9,
    }));
    let db = FakeDb::answering(Some(table_of(BETA)));
    let temp_lookup: &dyn TempLookup = &temp;
    let db_lookup: &dyn DbLookup = &db;
    let error = resolve_server(
        &loaded,
        target("test-b"),
        Some(temp_lookup),
        Some(db_lookup),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ResolveError::Build(_)));
    assert!(db.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_database_error_propagates() {
    let loaded = registry(&[BETA], &[]);
    let temp = FakeTemp::answering(None);
    let db = FakeDb::failing(ResolveError::from(McpTableError::ServerNotFound {
        server_id: "test-b".to_owned(),
    }));
    let temp_lookup: &dyn TempLookup = &temp;
    let db_lookup: &dyn DbLookup = &db;
    let error = resolve_server(
        &loaded,
        target("test-b"),
        Some(temp_lookup),
        Some(db_lookup),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ResolveError::Table(_)));
}

#[tokio::test]
async fn resolution_registry_needs_the_id_ip() {
    let loaded = registry_with(settings(&[], Some(&["203.0.113.0/24"])), &[PRIVATE], &[]);
    let blocked = ResolveTarget {
        server_id: "test-p",
        id_client_ip: Some("8.8.8.8"),
        name_client_ip: None,
        match_name: false,
    };
    assert!(
        resolve_server(&loaded, blocked, None, None)
            .await
            .unwrap()
            .is_none()
    );
    let allowed = ResolveTarget {
        server_id: "test-p",
        id_client_ip: Some("203.0.113.7"),
        name_client_ip: None,
        match_name: false,
    };
    let found = resolve_server(&loaded, allowed, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.source, ResolutionSource::Registry);
}

#[tokio::test]
async fn resolution_without_id_ip_reaches_a_private_server() {
    let loaded = registry(&[PRIVATE], &[]);
    let found = resolve_server(&loaded, target("test-p"), None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.table.server_id, "test-p");
}

#[tokio::test]
async fn resolution_name_match_is_off_by_default() {
    let loaded = registry(&[BETA], &[]);
    let named = ResolveTarget {
        server_id: "Beta",
        id_client_ip: None,
        name_client_ip: None,
        match_name: true,
    };
    let found = resolve_server(&loaded, named, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.source, ResolutionSource::Registry);
    assert_eq!(found.table.server_id, "test-b");
    assert!(
        resolve_server(&loaded, target("Beta"), None, None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn resolution_name_match_uses_the_name_ip() {
    let loaded = registry_with(
        settings(&[], Some(&["203.0.113.0/24"])),
        &[NAMED_PRIVATE],
        &[],
    );
    let blocked = ResolveTarget {
        server_id: "test-named",
        id_client_ip: None,
        name_client_ip: Some("8.8.8.8"),
        match_name: true,
    };
    assert!(
        resolve_server(&loaded, blocked, None, None)
            .await
            .unwrap()
            .is_none()
    );
    let allowed = ResolveTarget {
        server_id: "test-named",
        id_client_ip: None,
        name_client_ip: Some("203.0.113.7"),
        match_name: true,
    };
    let found = resolve_server(&loaded, allowed, None, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.table.server_id, "test-n");
}

#[tokio::test]
async fn resolution_authorize_missing_is_forbidden_by_default() {
    let access = FakeAccess::allowing(true);
    let error = authorize_server(
        None,
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap_err();
    match error {
        AuthorizeError::Forbidden(detail) => {
            assert_eq!(
                detail.get("detail").map(String::as_str),
                Some("Access denied")
            );
        }
        AuthorizeError::NotFound(_) | AuthorizeError::Access(_) => {
            panic!("expected a forbidden denial")
        }
    }
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_authorize_missing_is_not_found_when_configured() {
    let access = FakeAccess::allowing(true);
    let error = authorize_server(
        None,
        &auth(),
        &access,
        options(NonAdminMissing::NotFound),
        details(),
    )
    .await
    .unwrap_err();
    match error {
        AuthorizeError::NotFound(detail) => {
            assert_eq!(
                detail.get("detail").map(String::as_str),
                Some("Server not found")
            );
        }
        AuthorizeError::Forbidden(_) | AuthorizeError::Access(_) => {
            panic!("expected a missing denial")
        }
    }
}

#[tokio::test]
async fn resolution_authorize_missing_is_not_found_for_admins() {
    let access = FakeAccess::allowing(true);
    let admin = AuthorizeOptions {
        is_admin_view: true,
        allow_catalog_view: false,
        non_admin_missing: NonAdminMissing::Forbidden,
    };
    let error = authorize_server(None, &auth(), &access, admin, details())
        .await
        .unwrap_err();
    assert!(matches!(error, AuthorizeError::NotFound(_)));
}

#[tokio::test]
async fn resolution_authorize_admin_gets_a_temp_resolution() {
    let access = FakeAccess::allowing(false);
    let admin = AuthorizeOptions {
        is_admin_view: true,
        allow_catalog_view: false,
        non_admin_missing: NonAdminMissing::Forbidden,
    };
    let authorized = authorize_server(
        Some(temp_resolution(TEMP)),
        &auth(),
        &access,
        admin,
        details(),
    )
    .await
    .unwrap();
    assert_eq!(authorized.source, ResolutionSource::Temp);
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_authorize_temp_is_forbidden_for_non_admins() {
    let access = FakeAccess::allowing(true);
    let error = authorize_server(
        Some(temp_resolution(TEMP)),
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AuthorizeError::Forbidden(_)));
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_authorize_denies_without_access() {
    let access = FakeAccess::allowing(false);
    let error = authorize_server(
        Some(registry_resolution(BETA)),
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AuthorizeError::Forbidden(_)));
    assert_eq!(access.asked_ids(), vec!["test-b"]);
}

#[tokio::test]
async fn resolution_authorize_allows_with_access() {
    let access = FakeAccess::allowing(true);
    let authorized = authorize_server(
        Some(registry_resolution(BETA)),
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap();
    assert_eq!(authorized.table.server_id, "test-b");
    assert_eq!(access.asked_ids(), vec!["test-b"]);
}

#[tokio::test]
async fn resolution_authorize_catalog_view_skips_the_check() {
    let access = FakeAccess::allowing(false);
    let catalog = AuthorizeOptions {
        is_admin_view: false,
        allow_catalog_view: true,
        non_admin_missing: NonAdminMissing::Forbidden,
    };
    let authorized = authorize_server(
        Some(registry_resolution(BETA)),
        &auth(),
        &access,
        catalog,
        details(),
    )
    .await
    .unwrap();
    assert_eq!(authorized.table.server_id, "test-b");
    assert!(access.asked_ids().is_empty());
}

#[tokio::test]
async fn resolution_authorize_access_error_is_not_a_denial() {
    let access = FakeAccess::failing();
    let error = authorize_server(
        Some(registry_resolution(BETA)),
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AuthorizeError::Access(_)));
}

#[tokio::test]
async fn resolution_authorize_checks_the_resolved_id() {
    let loaded = registry(&[BETA], &[]);
    let named = ResolveTarget {
        server_id: "Beta",
        id_client_ip: None,
        name_client_ip: None,
        match_name: true,
    };
    let resolved = resolve_server(&loaded, named, None, None)
        .await
        .unwrap()
        .unwrap();
    let access = FakeAccess::allowing(true);
    let authorized = authorize_server(
        Some(resolved),
        &auth(),
        &access,
        options(NonAdminMissing::Forbidden),
        details(),
    )
    .await
    .unwrap();
    assert_eq!(authorized.table.server_id, "test-b");
    assert_eq!(access.asked_ids(), vec!["test-b"]);
}
