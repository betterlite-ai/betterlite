#![cfg(test)]

use axum::http::StatusCode;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::mcp::table_access::types::McpTableError;
use betterlite_mcp::registry::resolution::ResolveError;

use super::ports::{SessionIds as _, UuidSessionIds};
use crate::mcp::oauth_sessions::SessionError;
use crate::mcp::oauth_sessions::tests::testing::{
    AccessOutcome, BuilderChoice, DatabaseFault, HarnessConfig, RedisFault, alias_server,
    auth_with, draft_row, harness, request, row, row_named, sealed_server, server, server_named,
    temporary_key,
};
use crate::mcp::server_writes::ServerWriteError;

const SESSION: &str =
    r#"{"server_id":"test-session-1","url":"https://mcp.example.test/sse","transport":"sse"}"#;

#[tokio::test]
async fn the_local_table_serves_a_session_until_its_last_second() {
    let harness = harness(HarnessConfig::new());
    let cached = harness.store.cache_local(server("test-session-1"), 300);
    assert_eq!(cached.fields.server_id, "test-session-1");
    harness.clock.advance(299);
    let live = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the local hit has no error");
    assert!(live.is_some());
    harness.clock.advance(1);
    let expired = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("an expired session is a miss");
    assert!(expired.is_none(), "an entry dies at its expiry instant");
}

#[tokio::test]
async fn a_non_positive_ttl_is_raised_to_one_second() {
    let harness = harness(HarnessConfig::new());
    let cached = harness.store.cache_local(server("test-session-1"), 0);
    assert_eq!(cached.fields.server_id, "test-session-1");
    let live = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the hit has no error");
    assert!(live.is_some(), "a ttl of 0 still lives for one second");
    harness.clock.advance(1);
    let expired = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the second is over");
    assert!(expired.is_none());
}

#[tokio::test]
async fn storing_the_same_id_again_refreshes_its_lifetime() {
    let harness = harness(HarnessConfig::new());
    let first = harness.store.cache_local(server("test-session-1"), 300);
    assert_eq!(first.fields.server_id, "test-session-1");
    harness.clock.advance(200);
    let refreshed = harness.store.cache_local(server("test-session-1"), 300);
    assert_eq!(refreshed.fields.server_id, "test-session-1");
    harness.clock.advance(200);
    let live = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the refreshed hit has no error");
    assert!(live.is_some(), "the second write moved the expiry out");
    harness.clock.advance(100);
    let expired = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the refreshed window is over");
    assert!(expired.is_none());
}

#[tokio::test]
async fn the_local_table_wins_over_the_draft_row_and_the_redis_copy() {
    let harness = harness(
        HarnessConfig::new()
            .with_database(DatabaseFault::None)
            .with_redis(RedisFault::None),
    );
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(row_named("test-session-1", "from-draft"));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = sealed_server(
        &harness.cipher,
        &server_named("test-session-1", "from-redis"),
    );
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let cached = harness
        .store
        .cache_local(server_named("test-session-1", "from-local"), 300);
    assert_eq!(cached.fields.name, "from-local");
    let hit = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the lookup has no error")
        .expect("a source serves the session");
    assert_eq!(hit.fields.name, "from-local");
}

#[tokio::test]
async fn a_draft_row_serves_a_lookup_the_local_table_misses() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(row_named("test-session-1", "from-draft"));
    let hit = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the draft read succeeds")
        .expect("the draft serves the session");
    assert_eq!(hit.fields.name, "from-draft");
    database.remove_draft("test-session-1");
    let gone = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the miss has no error");
    assert!(
        gone.is_none(),
        "a draft hit is never copied into the local table"
    );
}

#[tokio::test]
async fn a_redis_copy_serves_a_lookup_the_local_table_and_draft_miss() {
    let harness = harness(HarnessConfig::new().with_redis(RedisFault::None));
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = sealed_server(
        &harness.cipher,
        &server_named("test-session-1", "from-redis"),
    );
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let hit = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the lookup has no error")
        .expect("the Redis copy serves the session");
    assert_eq!(hit.fields.name, "from-redis");
    redis.clear();
    let gone = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await
        .expect("the miss has no error");
    assert!(
        gone.is_none(),
        "a Redis hit is never copied into the local table"
    );
}

#[tokio::test]
async fn a_draft_read_error_reaches_the_caller_even_when_redis_holds_a_copy() {
    let harness = harness(
        HarnessConfig::new()
            .with_database(DatabaseFault::DraftRead)
            .with_redis(RedisFault::None),
    );
    let redis = harness.redis.as_ref().expect("the backend is set");
    let sealed = sealed_server(&harness.cipher, &server("test-session-1"));
    redis.put(&temporary_key("test-session-1"), CacheValue::Text(sealed));
    let outcome = harness
        .store
        .get_cached_temporary_server("test-session-1")
        .await;
    assert!(matches!(outcome, Err(ResolveError::Table(_))));
}

#[tokio::test]
async fn draft_as_server_builds_the_row_with_encrypted_credentials() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(row("test-session-1"));
    let server = harness
        .store
        .draft_as_server("test-session-1")
        .await
        .expect("the draft builds")
        .expect("the row is live");
    assert_eq!(server.fields.server_id, "test-session-1");
    assert_eq!(
        harness.builder.calls(),
        vec![("test-session-1".to_owned(), true)]
    );
}

#[tokio::test]
async fn draft_as_server_is_none_without_a_database() {
    let harness = harness(HarnessConfig::new());
    let outcome = harness
        .store
        .draft_as_server("test-session-1")
        .await
        .expect("no database is a miss");
    assert!(outcome.is_none());
    assert!(harness.builder.calls().is_empty());
}

#[tokio::test]
async fn draft_as_server_is_none_for_a_missing_row() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let outcome = harness
        .store
        .draft_as_server("test-session-1")
        .await
        .expect("a missing row is a miss");
    assert!(outcome.is_none());
}

#[tokio::test]
async fn draft_as_server_reports_a_build_failure() {
    let harness = harness(
        HarnessConfig::new()
            .with_database(DatabaseFault::None)
            .with_builder(BuilderChoice::Failing),
    );
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(row("test-session-1"));
    let outcome = harness.store.draft_as_server("test-session-1").await;
    assert!(matches!(outcome, Err(ResolveError::Build(_))));
}

#[tokio::test]
async fn persist_draft_writes_nothing_without_a_database() {
    let harness = harness(HarnessConfig::new());
    harness
        .store
        .persist_draft(&request(SESSION), "test-session-1", "admin-1")
        .await
        .expect("no database means no write");
}

#[tokio::test]
async fn persist_draft_writes_the_session_with_ttl_300_and_the_given_id() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    harness
        .store
        .persist_draft(&request(SESSION), "test-session-1", "admin-1")
        .await
        .expect("the draft writes");
    let database = harness.database.as_ref().expect("the database is set");
    let created = database.created();
    assert_eq!(created.len(), 1);
    let (recorded, options) = created.first().expect("one draft write");
    assert_eq!(options.ttl_seconds, 300);
    assert_eq!(options.server_id.as_deref(), Some("test-session-1"));
    assert_eq!(options.touched_by, "admin-1");
    assert_eq!(recorded.server_id.as_deref(), Some("test-session-1"));
}

#[tokio::test]
async fn persist_draft_returns_the_draft_write_error() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::DraftWrite));
    let outcome = harness
        .store
        .persist_draft(&request(SESSION), "test-session-1", "admin-1")
        .await;
    assert!(matches!(
        outcome,
        Err(SessionError::Write(ServerWriteError::InsertedRowMissing))
    ));
}

#[test]
fn uuid_session_ids_mint_distinct_v4_ids() {
    let ids = UuidSessionIds;
    let first = ids.fresh_id();
    let second = ids.fresh_id();
    assert_ne!(first, second);
    let parsed = uuid::Uuid::parse_str(&first).expect("a fresh id is a uuid");
    assert_eq!(parsed.get_version_num(), 4);
}

#[tokio::test]
async fn resolve_session_id_mints_a_fresh_id_when_none_is_supplied() {
    let harness = harness(HarnessConfig::new());
    let registry = harness.registry.read().await;
    let body = r#"{"url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let id = harness
        .store
        .resolve_session_id(&request(body), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "fresh-session-1");
}

#[tokio::test]
async fn resolve_session_id_mints_a_fresh_id_for_an_empty_id() {
    let harness = harness(HarnessConfig::new());
    let registry = harness.registry.read().await;
    let body = r#"{"server_id":"","url":"https://mcp.example.test/sse","transport":"sse"}"#;
    let id = harness
        .store
        .resolve_session_id(&request(body), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "fresh-session-1");
}

#[tokio::test]
async fn resolve_session_id_keeps_an_id_the_registry_knows() {
    let harness = harness(
        HarnessConfig::new()
            .with_database(DatabaseFault::None)
            .with_servers(vec![server("saved-1")]),
    );
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(draft_row("saved-1"));
    let registry = harness.registry.read().await;
    let id = harness
        .store
        .resolve_session_id(&request(r#"{"server_id":"saved-1"}"#), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "saved-1");
}

#[tokio::test]
async fn resolve_session_id_keeps_the_supplied_id_without_a_database() {
    let harness = harness(HarnessConfig::new());
    let registry = harness.registry.read().await;
    let id = harness
        .store
        .resolve_session_id(&request(SESSION), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "test-session-1");
}

#[tokio::test]
async fn resolve_session_id_keeps_an_id_that_names_a_saved_row() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let database = harness.database.as_ref().expect("the database is set");
    database.put_server(row("test-session-1"));
    let registry = harness.registry.read().await;
    let id = harness
        .store
        .resolve_session_id(&request(SESSION), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "test-session-1");
}

#[tokio::test]
async fn resolve_session_id_mints_a_fresh_id_for_a_draft_row() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let database = harness.database.as_ref().expect("the database is set");
    database.put_draft(draft_row("test-session-1"));
    let registry = harness.registry.read().await;
    let id = harness
        .store
        .resolve_session_id(&request(SESSION), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "fresh-session-1");
}

#[tokio::test]
async fn resolve_session_id_mints_a_fresh_id_for_a_missing_row() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::None));
    let registry = harness.registry.read().await;
    let id = harness
        .store
        .resolve_session_id(&request(SESSION), &registry)
        .await
        .expect("the id resolves");
    assert_eq!(id, "fresh-session-1");
}

#[tokio::test]
async fn resolve_session_id_returns_the_database_error() {
    let harness = harness(HarnessConfig::new().with_database(DatabaseFault::ServerRead));
    let registry = harness.registry.read().await;
    let outcome = harness
        .store
        .resolve_session_id(&request(SESSION), &registry)
        .await;
    assert!(matches!(
        outcome,
        Err(SessionError::Table(McpTableError::MissingServerId))
    ));
}

#[tokio::test]
async fn an_unknown_id_is_not_found() {
    let harness = harness(HarnessConfig::new());
    let auth = auth_with(Some("admin-1"), "proxy_admin");
    let registry = harness.registry.read().await;
    let outcome = harness
        .store
        .cached_temporary_server_or_404("missing", &auth, None, &registry)
        .await;
    let Err(error) = outcome else {
        panic!("an unknown id is not found");
    };
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    let SessionError::NotFound(detail) = error else {
        panic!("the error carries the not-found detail map");
    };
    assert_eq!(
        detail.get("error"),
        Some(&"MCP server missing not found".to_owned())
    );
}

#[tokio::test]
async fn a_temporary_session_is_forbidden_for_a_caller_who_is_not_an_admin() {
    let harness = harness(HarnessConfig::new());
    let cached = harness.store.cache_local(server("test-session-1"), 300);
    assert_eq!(cached.fields.server_id, "test-session-1");
    let auth = auth_with(Some("user-1"), "internal_user");
    let registry = harness.registry.read().await;
    let outcome = harness
        .store
        .cached_temporary_server_or_404("test-session-1", &auth, None, &registry)
        .await;
    let Err(error) = outcome else {
        panic!("a temporary session is admin-only");
    };
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    let SessionError::Forbidden(detail) = error else {
        panic!("the error carries the forbidden detail map");
    };
    assert_eq!(
        detail.get("error"),
        Some(&"Access denied to MCP server test-session-1".to_owned())
    );
}

#[tokio::test]
async fn a_temporary_session_serves_an_admin_view() {
    let harness = harness(HarnessConfig::new());
    let cached = harness.store.cache_local(server("test-session-1"), 300);
    assert_eq!(cached.fields.server_id, "test-session-1");
    let auth = auth_with(None, "proxy_admin_viewer");
    let registry = harness.registry.read().await;
    let server = harness
        .store
        .cached_temporary_server_or_404("test-session-1", &auth, None, &registry)
        .await
        .expect("an admin view gets the session");
    assert_eq!(server.fields.server_id, "test-session-1");
}

#[tokio::test]
async fn a_saved_server_serves_a_caller_the_access_check_allows() {
    let harness = harness(
        HarnessConfig::new()
            .with_servers(vec![server("saved-1")])
            .with_access(AccessOutcome::Allow),
    );
    let auth = auth_with(Some("user-1"), "internal_user");
    let registry = harness.registry.read().await;
    let server = harness
        .store
        .cached_temporary_server_or_404("saved-1", &auth, None, &registry)
        .await
        .expect("the access check allows the caller");
    assert_eq!(server.fields.server_id, "saved-1");
}

#[tokio::test]
async fn a_saved_server_is_forbidden_when_the_access_check_denies() {
    let harness = harness(
        HarnessConfig::new()
            .with_servers(vec![server("saved-1")])
            .with_access(AccessOutcome::Deny),
    );
    let auth = auth_with(Some("user-1"), "internal_user");
    let registry = harness.registry.read().await;
    let outcome = harness
        .store
        .cached_temporary_server_or_404("saved-1", &auth, None, &registry)
        .await;
    let Err(error) = outcome else {
        panic!("the access check denies the caller");
    };
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_access_check_error_keeps_its_own_variant() {
    let harness = harness(
        HarnessConfig::new()
            .with_servers(vec![server("saved-1")])
            .with_access(AccessOutcome::Fail),
    );
    let auth = auth_with(Some("user-1"), "internal_user");
    let registry = harness.registry.read().await;
    let outcome = harness
        .store
        .cached_temporary_server_or_404("saved-1", &auth, None, &registry)
        .await;
    let Err(error) = outcome else {
        panic!("the access check fails");
    };
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, SessionError::Access(_)));
}

#[tokio::test]
async fn the_request_ip_gates_the_name_lookup() {
    let harness = harness(HarnessConfig::new().with_servers(vec![alias_server(
        "test-session-1",
        "test-alias",
        false,
    )]));
    let auth = auth_with(None, "proxy_admin");
    let registry = harness.registry.read().await;
    let blocked = harness
        .store
        .cached_temporary_server_or_404("test-alias", &auth, Some("8.8.8.8"), &registry)
        .await;
    assert!(
        blocked.is_err(),
        "a public ip cannot reach an internal server"
    );
    let allowed = harness
        .store
        .cached_temporary_server_or_404("test-alias", &auth, None, &registry)
        .await
        .expect("no ip means the name lookup passes");
    assert_eq!(allowed.fields.alias.as_deref(), Some("test-alias"));
}
