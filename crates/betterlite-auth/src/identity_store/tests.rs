#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::budget_permission_rows::JsonData;
use opentelemetry::trace::SpanContext;
use rstest::rstest;
use rust_decimal::Decimal;

use super::IdentityStore;
use super::error::IdentityStoreError;
use super::ports::{KeyObjectCache, Ports};
use super::principal::{Projection, principal_from_key};
use super::rows;
use crate::auth_cache::{WriteOptions, gateway_cache};
use crate::auth_errors::conversion::as_proxy_exception;
use crate::auth_identity::records::{NetworkContext, SourceKey, TeamIdentity};
use crate::auth_identity::roles::{AuthMethod, PrincipalType, Role, TeamRole};
use crate::auth_key_lookup::cache::KeyCacheWriter;
use crate::auth_key_lookup::error::KeyLookupError;
use crate::auth_key_lookup::permission::KeyPermissionLookup;
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::resolved_key::auth::AuthError;

mod support;
use support::{
    CacheAnswer, FakeCache, FakePermissions, FakeSource, FakeWriter, KEY_ROW, Logs, StoreSetup,
    auth_from_fields, permission_table, reader, setup, span, view_from_text,
};

const NO_DATABASE: &str = "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys";
const CACHED_FIELDS: &str = r#"{"token":"hashed-token","key_name":"key-1","user_id":"u1","budget_reservation":{"reserved":1},"budget_throttle_pct":0.5,"request_route":"/v1/chat/completions","parent_otel_span":{"trace_id":"abc"}}"#;

#[tokio::test]
async fn cache_copy_isolation() {
    let cache = FakeCache::new(CacheAnswer::Hit(Box::new(auth_from_fields(CACHED_FIELDS))));
    let source = FakeSource::new(Vec::new());
    let permissions = FakePermissions::new(Vec::new());
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        cache: Arc::clone(&cache),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert!(key.extra.budget_reservation.is_none());
    assert!(key.extra.budget_throttle_pct.is_none());
    assert!(key.extra.request_route.is_none());
    assert!(key.extra.parent_otel_span.is_none());
    assert_eq!(key.view.token.key_name.as_deref(), Some("key-1"));
    assert_eq!(cache.reads(), vec!["hashed-token".to_owned()]);
    let held = cache.hit().expect("the fake still holds the cached object");
    assert!(held.extra.budget_reservation.is_some());
    assert!(held.extra.budget_throttle_pct.is_some());
    assert!(held.extra.request_route.is_some());
    assert!(held.extra.parent_otel_span.is_some());
    assert!(source.calls().is_empty());
    assert!(permissions.calls().is_empty());
    assert!(writer.writes().is_empty());
}

#[rstest]
#[case::open(false)]
#[case::cache_only(true)]
#[tokio::test]
async fn no_db_before_cache(#[case] check_cache_only: bool) {
    let cache = FakeCache::new(CacheAnswer::Hit(Box::new(auth_from_fields(KEY_ROW))));
    let store = StoreSetup {
        cache: Arc::clone(&cache),
        check_cache_only,
        ..setup()
    }
    .store();
    let error = store.resolve_key("hashed-token").await.unwrap_err();
    assert_eq!(error.to_string(), NO_DATABASE);
    assert!(matches!(
        &error,
        IdentityStoreError::KeyLookup(KeyLookupError::NoDatabase)
    ));
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn resolve_rejects_an_absent_database() {
    let error = setup().store().resolve("hashed-token").await.unwrap_err();
    assert_eq!(error.to_string(), NO_DATABASE);
}

#[tokio::test]
async fn cache_only_miss() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        cache: FakeCache::new(CacheAnswer::Miss),
        permissions: FakePermissions::new(Vec::new()),
        writer: Arc::clone(&writer),
        span: None,
        check_cache_only: true,
    }
    .store();
    let error = store.resolve_key("hashed-token").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Key doesn't exist in cache + check_cache_only=True. key=hashed-token."
    );
    assert!(matches!(
        &error,
        IdentityStoreError::KeyLookup(KeyLookupError::NotInCache { .. })
    ));
    assert!(source.calls().is_empty());
    assert!(writer.writes().is_empty());

    let open_source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let open_writer = FakeWriter::new(false);
    let open_store = StoreSetup {
        reader: Some(reader(Arc::clone(&open_source))),
        writer: Arc::clone(&open_writer),
        ..setup()
    }
    .store();
    open_store.resolve_key("hashed-token").await.unwrap();
    assert_eq!(open_source.calls().len(), 1);
    assert_eq!(open_writer.writes(), vec!["hashed-token".to_owned()]);
}

#[tokio::test]
async fn cache_error_propagates() {
    let source = FakeSource::new(Vec::new());
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        cache: FakeCache::new(CacheAnswer::Error),
        ..setup()
    }
    .store();
    let error = store.resolve_key("hashed-token").await.unwrap_err();
    assert!(matches!(
        &error,
        IdentityStoreError::Cache(AuthError::NonStringApiKey)
    ));
    assert_eq!(
        error.to_string(),
        "api_key must be a string to compute its safe provenance hash"
    );
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn reader_arguments() {
    let passed_span = span(7);
    let source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        span: Some(passed_span.clone()),
        ..setup()
    }
    .store();
    store.resolve_key("hashed-token").await.unwrap();
    let calls = source.calls();
    let Some((token, use_writer, recorded_span)) = calls.first() else {
        panic!("the reader must read the source once");
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(token, "hashed-token");
    assert!(!*use_writer);
    assert_eq!(
        recorded_span.as_ref().map(SpanContext::trace_id),
        Some(passed_span.trace_id())
    );
}

#[tokio::test]
async fn row_validation_before_permissions() {
    let rejected: BTreeMap<String, JsonData> = serde_json::from_str(r#"{"token":5}"#).unwrap();
    assert!(matches!(
        rows::auth_from_fields(rejected),
        Err(KeyLookupError::Row(_))
    ));

    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1"}"#,
    )))]);
    let permissions = FakePermissions::new(vec![Ok(Some(permission_table()))]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert!(key.view.token.object_permission.is_some());
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), false)]);
    assert_eq!(writer.seen_permission(), vec![true]);
    assert_eq!(source.calls().len(), 1);
}

#[tokio::test]
async fn permission_failure_debug_then_cache() {
    let (logs, _guard) = Logs::capture();
    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1"}"#,
    )))]);
    let permissions = FakePermissions::new(vec![Err(PermissionError::MissingInDatabaseOnly)]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert!(key.view.token.object_permission.is_none());
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), false)]);
    assert_eq!(writer.writes(), vec!["hashed-token".to_owned()]);
    assert_eq!(writer.seen_permission(), vec![false]);
    assert!(logs.text().contains(
        "Failed to load object_permission for key with object_permission_id=op-1: Referenced object permission does not exist"
    ));
}

#[rstest]
#[case::carried_permission(
    r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1","object_permission":{"object_permission_id":"op-1","models":["gpt-4"]}}"#,
    true
)]
#[case::no_permission_id(r#"{"token":"hashed-token","user_id":"u1"}"#, false)]
#[case::empty_permission_id(
    r#"{"token":"hashed-token","user_id":"u1","object_permission_id":""}"#,
    false
)]
#[tokio::test]
async fn permission_skip_and_none(#[case] row: &str, #[case] carries_permission: bool) {
    let permissions = FakePermissions::new(vec![Ok(Some(permission_table()))]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(FakeSource::new(vec![Ok(Some(view_from_text(row)))]))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert!(permissions.calls().is_empty());
    assert_eq!(writer.writes(), vec!["hashed-token".to_owned()]);
    assert_eq!(
        key.view.token.object_permission.is_some(),
        carries_permission
    );
    assert_eq!(writer.seen_permission(), vec![carries_permission]);
}

#[tokio::test]
async fn writer_mutation_visible() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1","last_refreshed_at":5}"#,
    )))]);
    let permissions = FakePermissions::new(vec![Ok(Some(permission_table()))]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert_eq!(key.view.last_refreshed_at, Some(Decimal::ONE));
    assert!(key.view.token.object_permission.is_some());
    assert_eq!(writer.writes(), vec!["hashed-token".to_owned()]);
    assert_eq!(writer.seen_permission(), vec![true]);
}

#[tokio::test]
async fn writer_failure_propagates() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1"}"#,
    )))]);
    let permissions = FakePermissions::new(vec![Ok(Some(permission_table()))]);
    let writer = FakeWriter::new(true);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let error = store.resolve_key("hashed-token").await.unwrap_err();
    assert!(matches!(
        &error,
        IdentityStoreError::KeyLookup(KeyLookupError::CacheWrite(_))
    ));
    assert_eq!(error.to_string(), "the key cache write failed");
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), false)]);
    assert_eq!(writer.writes(), vec!["hashed-token".to_owned()]);
}

#[tokio::test]
async fn carrier_roundtrip_and_missing_source_key() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        ..setup()
    }
    .store();
    let principal = store.resolve("hashed-token").await.unwrap();
    assert_eq!(principal.subject, "u1");
    assert_eq!(
        principal.credential_ref.token_id.as_deref(),
        Some("hashed-token")
    );
    assert!(principal.credential_ref.key_id.is_none());
    assert_eq!(principal.auth_method, AuthMethod::ApiKey);
    let carried = IdentityStore::key_from_principal(&principal).unwrap();
    assert_eq!(carried.view.token.key_name.as_deref(), Some("key-1"));

    let mut stripped = principal;
    stripped.source_key = SourceKey(None);
    let error = IdentityStore::key_from_principal(&stripped).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Principal carries no source key; it was not produced by IdentityStore.resolve"
    );
    assert!(matches!(
        &error,
        IdentityStoreError::PrincipalMissingSourceKey
    ));
}

const fn projection(subject_fallback: Option<&'static str>) -> Projection<'static> {
    Projection {
        auth_method: AuthMethod::ApiKey,
        issuer: None,
        subject_fallback,
        scopes: &[],
        credential_ref: None,
        network: None,
    }
}

#[tokio::test]
async fn resolve_uses_the_stored_token_over_the_requested_hash() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"stored-token","key_name":"key-1"}"#,
    )))]);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        ..setup()
    }
    .store();
    let principal = store.resolve("requested-hash").await.unwrap();
    assert_eq!(principal.subject, "stored-token");
    assert_eq!(
        principal.credential_ref.token_id.as_deref(),
        Some("stored-token")
    );
    let calls = source.calls();
    assert_eq!(
        calls.first().map(|(token, _, _)| token.as_str()),
        Some("requested-hash")
    );
}

#[tokio::test]
async fn resolve_as_carries_the_auth_method_and_network() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        ..setup()
    }
    .store();
    let network = NetworkContext {
        client_ip: Some("203.0.113.7".to_owned()),
        host: None,
        via_trusted_proxy: true,
    };
    let principal = store
        .resolve_as("hashed-token", AuthMethod::BearerJwt, Some(network.clone()))
        .await
        .unwrap();
    assert_eq!(principal.auth_method, AuthMethod::BearerJwt);
    assert_eq!(principal.network, network);
}

#[tokio::test]
async fn permission_none_stays_absent() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(
        r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1"}"#,
    )))]);
    let permissions = FakePermissions::new(vec![Ok(None)]);
    let writer = FakeWriter::new(false);
    let store = StoreSetup {
        reader: Some(reader(Arc::clone(&source))),
        permissions: Arc::clone(&permissions),
        writer: Arc::clone(&writer),
        ..setup()
    }
    .store();
    let key = store.resolve_key("hashed-token").await.unwrap();
    assert!(key.view.token.object_permission.is_none());
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), false)]);
    assert_eq!(writer.seen_permission(), vec![false]);
}

#[test]
fn cache_only_failure_keeps_the_auth_error_envelope() {
    let failure = IdentityStoreError::KeyLookup(KeyLookupError::NotInCache {
        key: "hashed-token".to_owned(),
    })
    .failure();
    let client = as_proxy_exception(&failure);
    let exception = client.exception();
    assert_eq!(
        exception.message,
        "Authentication Error, Key doesn't exist in cache + check_cache_only=True. key=hashed-token."
    );
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
}

#[test]
fn debug_names_the_store() {
    let store = setup().store();
    assert_eq!(
        format!("{store:?}"),
        "IdentityStore { check_cache_only: false, .. }"
    );
}

#[test]
fn ports_require_assembles_the_dependencies() {
    let cache: Arc<dyn KeyObjectCache> = FakeCache::new(CacheAnswer::Miss);
    let permissions: Arc<dyn KeyPermissionLookup> = FakePermissions::new(Vec::new());
    let writer: Arc<dyn KeyCacheWriter> = FakeWriter::new(false);
    let ports = Ports {
        reader: None,
        cache: Some(cache),
        permissions: Some(permissions),
        writer: Some(writer),
    };
    ports.require().unwrap();
    let error = Ports::default()
        .require()
        .err()
        .expect("incomplete ports fail");
    assert_eq!(
        error.to_string(),
        "the identity store is missing required ports: cache, permissions, writer"
    );
}

#[test]
fn projection_reads_principal_type() {
    let human_key =
        auth_from_fields(r#"{"token":"hashed-token","key_name":"key-1","user_id":"u1"}"#);
    let human = principal_from_key(&human_key, projection(None));
    assert_eq!(human.principal_type, PrincipalType::Human);
    assert_eq!(human.user.expect("the user record is present").id, "u1");

    let service_key =
        auth_from_fields(r#"{"token":"hashed-token","key_name":"key-1","user_id":""}"#);
    let service_account = principal_from_key(&service_key, projection(None));
    assert_eq!(
        service_account.principal_type,
        PrincipalType::ServiceAccount
    );
    assert_eq!(
        service_account
            .user
            .expect("the empty user record is present")
            .id,
        ""
    );
}

#[test]
fn projection_subject_precedence() {
    let user_and_alias = auth_from_fields(
        r#"{"token":"hashed-token","key_name":"key-1","user_id":"u1","key_alias":"alias-1"}"#,
    );
    assert_eq!(
        principal_from_key(&user_and_alias, projection(None)).subject,
        "u1"
    );

    let empty_user_and_alias = auth_from_fields(
        r#"{"token":"hashed-token","key_name":"key-1","user_id":"","key_alias":"alias-empty"}"#,
    );
    assert_eq!(
        principal_from_key(&empty_user_and_alias, projection(None)).subject,
        "alias-empty"
    );

    let without_candidates = auth_from_fields(r#"{"token":"hashed-token","key_name":"key-1"}"#);
    assert_eq!(
        principal_from_key(&without_candidates, projection(Some("fallback-subject"))).subject,
        "fallback-subject"
    );
    assert_eq!(
        principal_from_key(&without_candidates, projection(None)).subject,
        ""
    );
}

#[test]
fn projection_reads_teams_and_roles() {
    let admin = auth_from_fields(
        r#"{"token":"hashed-token","key_name":"key-1","team_id":"t1","team_alias":"team-one","team_member":{"user_id":"u1","role":"admin"},"user_role":"org_admin"}"#,
    );
    let projected = principal_from_key(&admin, projection(None));
    assert_eq!(
        projected.teams,
        vec![TeamIdentity {
            id: "t1".to_owned(),
            name: Some("team-one".to_owned()),
            role: TeamRole::Admin,
        }]
    );
    assert_eq!(projected.roles, vec![Role::OrgAdmin]);

    let plain = auth_from_fields(
        r#"{"token":"hashed-token","key_name":"key-1","team_id":"t2","user_role":"internal_user"}"#,
    );
    let member = principal_from_key(&plain, projection(None));
    assert_eq!(member.teams.len(), 1);
    let team = member.teams.first().expect("the team is present");
    assert_eq!(team.role, TeamRole::Member);
    assert!(member.roles.is_empty());
}

#[test]
fn null_row_fields_read_as_defaults() {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(r#"{"token":"hashed-token","team_models":null}"#).unwrap();
    let key = rows::auth_from_fields(fields).unwrap();
    assert!(key.view.team_models.is_empty());
}

#[tokio::test]
async fn cache_port_returns_the_stored_key() {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1000.0_f64);
    let ordinary =
        LocalMemoryOps::new(1024, Arc::clone(&clock)).expect("the local memory tier builds");
    let key_objects =
        LocalMemoryOps::new(1024, Arc::clone(&clock)).expect("the local memory tier builds");
    let cache = gateway_cache(
        Arc::new(ordinary),
        Arc::new(key_objects),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    );
    let hashed = "a".repeat(64);
    let key = auth_from_fields(KEY_ROW);
    cache
        .async_set(Some(hashed.as_str()), &key, WriteOptions::default())
        .await
        .unwrap();
    let found = cache.read_key_object(&hashed).await.unwrap();
    let stored = found.expect("the stored key object comes back");
    assert_eq!(stored.view.token.key_name.as_deref(), Some("key-1"));
}
