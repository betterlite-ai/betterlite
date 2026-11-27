#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use opentelemetry::trace::SpanContext;
use rust_decimal::Decimal;

use super::IdentityStore;
use super::error::IdentityStoreError;
use super::rows;
use crate::auth_identity::records::SourceKey;
use crate::auth_identity::roles::AuthMethod;
use crate::auth_key_lookup::error::KeyLookupError;
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
    let store = StoreSetup {
        cache: Arc::clone(&cache),
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
}

#[tokio::test]
async fn no_db_before_cache() {
    for check_cache_only in [false, true] {
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

#[tokio::test]
async fn permission_skip_and_none() {
    let cases = [
        (
            r#"{"token":"hashed-token","user_id":"u1","object_permission_id":"op-1","object_permission":{"object_permission_id":"op-1","models":["gpt-4"]}}"#,
            true,
        ),
        (r#"{"token":"hashed-token","user_id":"u1"}"#, false),
        (
            r#"{"token":"hashed-token","user_id":"u1","object_permission_id":""}"#,
            false,
        ),
    ];
    for (row, carries_permission) in cases {
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
