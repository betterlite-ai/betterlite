#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::ErrorType;
use betterlite_db::identity_rows::Payload;
use rust_decimal::Decimal;

use super::error::KeyLookupError;
use super::jwt::{
    get_jwt_key_mapping_cache_keys_for_tokens, get_jwt_key_mapping_object,
    jwt_key_mapping_cache_key,
};
use super::key_object::get_key_object;
use super::reader::AUTH_RECONNECT_REASON;
use super::session::resolve_object_permission_for_unresolvable_team;
use super::{HashedKeyAuthenticator, KeyAuthenticator};
use crate::auth_errors::conversion::as_proxy_exception;
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::identity_grants::denial::PublicDenial;
use crate::user_roles::UserRole;

mod support;
use support::{
    FakeCache, FakeMappings, FakeMembers, FakePermissions, FakeSource, FakeTeams, FakeUsers,
    FakeWriter, KEY_ROW, UserAnswer, auth_from_fields, auth_text, fake_reconnect, fault,
    key_request, lookups, mapping, plain_session_auth, reader, reconnect_calls, refresh, team_row,
    team_session_auth, transport, user_row, view_from_text,
};

#[tokio::test]
async fn db_before_cache() {
    let cache = FakeCache::new(Some(auth_from_fields(KEY_ROW)));
    let lookups = lookups(None, Arc::clone(&cache), FakeWriter::new(false), None);
    let error = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyLookupError::NoDatabase));
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn hash_is_cache_key() {
    let cache = FakeCache::new(Some(auth_from_fields(KEY_ROW)));
    let lookups = Arc::new(lookups(
        Some(reader(FakeSource::new(Vec::new()), None)),
        Arc::clone(&cache),
        FakeWriter::new(false),
        None,
    ));
    let authenticator = HashedKeyAuthenticator { lookups };
    authenticator.authenticate("sk-raw").await.unwrap();
    assert_eq!(cache.reads().len(), 1);
    assert_ne!(cache.reads()[0], "sk-raw");
    assert_eq!(cache.reads()[0].len(), 64);
    authenticator.authenticate("plain").await.unwrap();
    assert_eq!(cache.reads()[1], "plain");
}

#[tokio::test]
async fn key_cache_copy() {
    let mut hit = auth_from_fields(KEY_ROW);
    hit.extra.budget_reservation = Some(Payload::default());
    hit.extra.budget_throttle_pct = Some(Decimal::ZERO);
    hit.extra.request_route = Some("/v1/chat/completions".to_owned());
    let lookups = lookups(
        Some(reader(FakeSource::new(Vec::new()), None)),
        FakeCache::new(Some(hit.clone())),
        FakeWriter::new(false),
        None,
    );
    let auth = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap();
    assert!(auth.extra.budget_reservation.is_none());
    assert!(auth.extra.budget_throttle_pct.is_none());
    assert!(auth.extra.request_route.is_none());
    assert!(hit.extra.budget_reservation.is_some());
    assert!(hit.extra.budget_throttle_pct.is_some());
}

#[tokio::test]
async fn both_flags_cache_miss() {
    let source = FakeSource::new(Vec::new());
    let cache = FakeCache::new(Some(auth_from_fields(KEY_ROW)));
    let lookups = lookups(
        Some(reader(Arc::clone(&source), None)),
        Arc::clone(&cache),
        FakeWriter::new(false),
        None,
    );
    let mut call = key_request("hashed-key");
    call.check_db_only = true;
    call.check_cache_only = Some(true);
    let error = get_key_object(&lookups, &call).await.unwrap_err();
    assert!(matches!(error, KeyLookupError::NotInCache { .. }));
    assert!(source.calls().is_empty());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn writer_combined_view() {
    let source = FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]);
    let cache = FakeCache::new(Some(auth_from_fields(KEY_ROW)));
    let writer = FakeWriter::new(false);
    let lookups = lookups(
        Some(reader(Arc::clone(&source), None)),
        Arc::clone(&cache),
        Arc::clone(&writer),
        None,
    );
    let mut call = key_request("hashed-key");
    call.check_db_only = true;
    let auth = get_key_object(&lookups, &call).await.unwrap();
    assert_eq!(source.calls(), vec![("hashed-key".to_owned(), true)]);
    assert_eq!(auth.view.token.key_name.as_deref(), Some("key-1"));
    assert!(cache.reads().is_empty());
    assert!(writer.writes().is_empty());
}

#[tokio::test]
async fn missing_key_401() {
    let lookups = lookups(
        Some(reader(FakeSource::new(vec![Ok(None)]), None)),
        FakeCache::new(None),
        FakeWriter::new(false),
        None,
    );
    let error = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyLookupError::MissingKey { .. }));
    let exception = as_proxy_exception(&error.failure()).exception().clone();
    assert_eq!(exception.code, "401");
    assert_eq!(exception.error_type, "token_not_found_in_db");
    assert_eq!(exception.param.as_deref(), Some("key"));
}

#[tokio::test]
async fn row_nulls_excluded() {
    let row = r#"{"token":"hashed-key","key_name":"key-1","team_alias":null,"max_budget":null}"#;
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(row)))]),
            None,
        )),
        FakeCache::new(None),
        FakeWriter::new(false),
        None,
    );
    let auth = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap();
    assert_eq!(auth.view.token.key_name.as_deref(), Some("key-1"));
    assert!(auth.view.team_alias.is_none());
    assert!(auth.view.token.max_budget.is_none());
    assert_eq!(auth.view.token.spend, Decimal::ZERO);
}

#[tokio::test]
async fn permission_reader_debug_drop() {
    let row = r#"{"token":"hashed-key","object_permission_id":"op-1"}"#;
    let permissions = FakePermissions::new(vec![Err(PermissionError::NoDatabase)]);
    let writer = FakeWriter::new(false);
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(row)))]),
            None,
        )),
        FakeCache::new(None),
        Arc::clone(&writer),
        Some(Arc::clone(&permissions)),
    );
    let auth = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap();
    assert!(auth.view.token.object_permission.is_none());
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), false)]);
    assert_eq!(writer.writes(), vec!["hashed-key".to_owned()]);
}

#[tokio::test]
async fn writer_permission_reload() {
    let row = r#"{"token":"hashed-key","object_permission_id":"op-1"}"#;
    let permissions = FakePermissions::new(vec![Ok(Some(permission_table()))]);
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(row)))]),
            None,
        )),
        FakeCache::new(None),
        FakeWriter::new(false),
        Some(Arc::clone(&permissions)),
    );
    let mut call = key_request("hashed-key");
    call.check_db_only = true;
    let auth = get_key_object(&lookups, &call).await.unwrap();
    let found = auth.view.token.object_permission.as_ref();
    assert_eq!(
        found.map(|table| table.object_permission_id.as_str()),
        Some("op-1")
    );
    assert_eq!(permissions.calls(), vec![("op-1".to_owned(), true)]);
}

#[tokio::test]
async fn writer_permission_error() {
    let row = r#"{"token":"hashed-key","object_permission_id":"op-1"}"#;
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(row)))]),
            None,
        )),
        FakeCache::new(None),
        FakeWriter::new(false),
        Some(FakePermissions::new(vec![Err(
            PermissionError::MissingInDatabaseOnly,
        )])),
    );
    let mut call = key_request("hashed-key");
    call.check_db_only = true;
    let error = get_key_object(&lookups, &call).await.unwrap_err();
    assert!(matches!(
        error,
        KeyLookupError::Permission(PermissionError::MissingInDatabaseOnly)
    ));
}

#[tokio::test]
async fn writer_does_not_cache() {
    let writer = FakeWriter::new(false);
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]),
            None,
        )),
        FakeCache::new(None),
        Arc::clone(&writer),
        None,
    );
    let mut call = key_request("hashed-key");
    call.check_db_only = true;
    get_key_object(&lookups, &call).await.unwrap();
    assert!(writer.writes().is_empty());
}

#[tokio::test]
async fn enrichment_before_cache() {
    let row = r#"{"token":"hashed-key","object_permission_id":"op-1"}"#;
    let writer = FakeWriter::new(false);
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(row)))]),
            None,
        )),
        FakeCache::new(None),
        Arc::clone(&writer),
        Some(FakePermissions::new(vec![Ok(Some(permission_table()))])),
    );
    get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap();
    assert_eq!(writer.seen_permission(), vec![true]);
}

#[tokio::test]
async fn cache_write_error_propagates() {
    let lookups = lookups(
        Some(reader(
            FakeSource::new(vec![Ok(Some(view_from_text(KEY_ROW)))]),
            None,
        )),
        FakeCache::new(None),
        FakeWriter::new(true),
        None,
    );
    let error = get_key_object(&lookups, &key_request("hashed-key"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyLookupError::CacheWrite(_)));
}

#[tokio::test(start_paused = true)]
async fn gate_and_retry_share_deadline() {
    let source = FakeSource::new(vec![Err(transport())]);
    let (mut reconnect, _calls) = fake_reconnect(Ok(true));
    reconnect.delay = Some(Duration::from_secs(30));
    let mut key_reader = reader(Arc::clone(&source), Some(reconnect));
    Arc::get_mut(&mut key_reader)
        .unwrap()
        .default_deadline_seconds = 0.05;
    let error = key_reader
        .fetch("hashed-key", false, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KeyLookupError::Lookup(DbLookupError::Deadline(_))
    ));
    assert_eq!(source.calls().len(), 1);
}

#[tokio::test]
async fn transport_reconnect_once() {
    let source = FakeSource::new(vec![Err(transport()), Ok(Some(view_from_text(KEY_ROW)))]);
    let (reconnect, calls) = fake_reconnect(Ok(true));
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    let found = key_reader.fetch("hashed-key", false, None).await.unwrap();
    assert!(found.is_some());
    assert_eq!(source.calls().len(), 2);
    let attempts = reconnect_calls(&calls);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, AUTH_RECONNECT_REASON);
    assert_eq!(attempts[0].1.to_bits(), 2.0_f64.to_bits());
    assert_eq!(attempts[0].2.to_bits(), 0.1_f64.to_bits());
}

#[tokio::test]
async fn data_fault_no_reconnect() {
    let source = FakeSource::new(vec![Err(fault(ErrorType::Data, "bad row"))]);
    let (reconnect, calls) = fake_reconnect(Ok(true));
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    let error = key_reader
        .fetch("hashed-key", false, None)
        .await
        .unwrap_err();
    assert!(matches!(error, KeyLookupError::Read(_)));
    assert_eq!(error.to_string(), "bad row");
    assert_eq!(source.calls().len(), 1);
    assert!(reconnect_calls(&calls).is_empty());
}

#[tokio::test]
async fn absent_reconnect_method() {
    let source = FakeSource::new(vec![Err(transport())]);
    let key_reader = reader(Arc::clone(&source), None);
    let error = key_reader
        .fetch("hashed-key", false, None)
        .await
        .unwrap_err();
    let KeyLookupError::Lookup(DbLookupError::Database(reported)) = error else {
        panic!("a transport fault without reconnect support is a database error");
    };
    assert_eq!(reported.error_type, ErrorType::Connect);
    assert_eq!(source.calls().len(), 1);
}

#[tokio::test]
async fn reconnect_false_rethrows() {
    let source = FakeSource::new(vec![Err(fault(ErrorType::Read, "first fault"))]);
    let (reconnect, calls) = fake_reconnect(Ok(false));
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    let error = key_reader
        .fetch("hashed-key", false, None)
        .await
        .unwrap_err();
    let KeyLookupError::Lookup(DbLookupError::Database(reported)) = error else {
        panic!("a false reconnect rethrows the first fault");
    };
    assert_eq!(reported.message, "first fault");
    assert_eq!(source.calls().len(), 1);
    assert_eq!(reconnect_calls(&calls).len(), 1);
}

#[tokio::test]
async fn reconnect_numeric_timeouts() {
    let source = FakeSource::new(vec![Err(transport()), Ok(None)]);
    let (mut reconnect, calls) = fake_reconnect(Ok(true));
    reconnect.timeout = Some(3.5);
    reconnect.lock = Some(0.25);
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    key_reader.fetch("hashed-key", false, None).await.unwrap();
    let attempts = reconnect_calls(&calls);
    assert_eq!(attempts[0].1.to_bits(), 3.5_f64.to_bits());
    assert_eq!(attempts[0].2.to_bits(), 0.25_f64.to_bits());
}

#[tokio::test]
async fn malformed_timeout_defaults() {
    let source = FakeSource::new(vec![Err(transport()), Ok(None)]);
    let (reconnect, calls) = fake_reconnect(Ok(true));
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    key_reader.fetch("hashed-key", false, None).await.unwrap();
    let attempts = reconnect_calls(&calls);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].0, AUTH_RECONNECT_REASON);
    assert_eq!(attempts[0].1.to_bits(), 2.0_f64.to_bits());
    assert_eq!(attempts[0].2.to_bits(), 0.1_f64.to_bits());
}

#[tokio::test]
async fn retry_error_propagates() {
    let source = FakeSource::new(vec![
        Err(transport()),
        Err(fault(ErrorType::Data, "retry fault")),
    ]);
    let (reconnect, _calls) = fake_reconnect(Ok(true));
    let key_reader = reader(Arc::clone(&source), Some(reconnect));
    let error = key_reader
        .fetch("hashed-key", false, None)
        .await
        .unwrap_err();
    assert!(matches!(error, KeyLookupError::Read(_)));
    assert_eq!(error.to_string(), "retry fault");
    assert_eq!(source.calls().len(), 2);
}

#[test]
fn global_and_scoped_keys() {
    assert_eq!(
        jwt_key_mapping_cache_key("group", "dev", None),
        "jwt_key_mapping:group:dev"
    );
    assert_eq!(
        jwt_key_mapping_cache_key("group", "dev", Some("issuer")),
        "jwt_key_mapping:issuer:group:dev"
    );
}

#[test]
fn empty_issuer_global() {
    assert_eq!(
        jwt_key_mapping_cache_key("group", "dev", Some("")),
        "jwt_key_mapping:group:dev"
    );
}

#[tokio::test]
async fn active_exact_scope() {
    let source = FakeMappings::new(
        Vec::new(),
        Some(mapping("hashed", "group", "dev", "issuer")),
    );
    let token = get_jwt_key_mapping_object("group", "dev", Some("issuer"), source.as_ref())
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("hashed"));
    let filter = source.filter().unwrap();
    assert_eq!(filter.claim_name, "group");
    assert_eq!(filter.claim_value, "dev");
    assert_eq!(filter.issuer, "issuer");
    assert!(filter.active);
}

#[tokio::test]
async fn no_implicit_global_fallback() {
    let source = FakeMappings::new(vec![mapping("global", "group", "dev", "")], None);
    let token = get_jwt_key_mapping_object("group", "dev", Some("issuer"), source.as_ref())
        .await
        .unwrap();
    assert!(token.is_none());
    assert_eq!(source.filter().unwrap().issuer, "issuer");
}

#[tokio::test]
async fn mapping_order_duplicates() {
    let source = FakeMappings::new(
        vec![
            mapping("a", "group", "dev", "issuer"),
            mapping("b", "team", "core", ""),
            mapping("c", "group", "dev", "issuer"),
        ],
        None,
    );
    let keys = get_jwt_key_mapping_cache_keys_for_tokens(&["hashed".to_owned()], source.as_ref())
        .await
        .unwrap();
    assert_eq!(
        keys,
        vec![
            "jwt_key_mapping:issuer:group:dev".to_owned(),
            "jwt_key_mapping:team:core".to_owned(),
            "jwt_key_mapping:issuer:group:dev".to_owned(),
        ]
    );
}

#[tokio::test]
async fn empty_batch_no_query() {
    let source = FakeMappings::new(Vec::new(), None);
    let keys = get_jwt_key_mapping_cache_keys_for_tokens(&[], source.as_ref())
        .await
        .unwrap();
    assert!(keys.is_empty());
    assert!(source.token_calls().is_empty());
}

#[tokio::test]
async fn live_role_replaces_minted() {
    let users = FakeUsers(UserAnswer::Found(Box::new(user_row(
        r#"{"user_id":"u1","teams":["t1"],"user_role":"proxy_admin"}"#,
    ))));
    let teams = FakeTeams(Some(team_row(r#"{"team_id":"t1"}"#)));
    let auth = refresh(&team_session_auth(), &users, &teams, &FakeMembers(None))
        .await
        .unwrap();
    assert_eq!(auth.extra.user_role, Some(UserRole::ProxyAdmin));
}

#[tokio::test]
async fn team_models_empty() {
    let users = FakeUsers(UserAnswer::Found(Box::new(user_row(
        r#"{"user_id":"u1","teams":["t1"],"models":["user-model"]}"#,
    ))));
    let teams = FakeTeams(Some(team_row(
        r#"{"team_id":"t1","models":["team-model"]}"#,
    )));
    let auth = refresh(&team_session_auth(), &users, &teams, &FakeMembers(None))
        .await
        .unwrap();
    assert!(auth.view.token.models.is_empty());
    assert_eq!(
        auth.view.team_models,
        vec![JsonData::String("team-model".to_owned())]
    );
}

#[tokio::test]
async fn live_user_models() {
    let users = FakeUsers(UserAnswer::Found(Box::new(user_row(
        r#"{"user_id":"u1","models":["user-model"]}"#,
    ))));
    let auth = refresh(
        &plain_session_auth(),
        &users,
        &FakeTeams(None),
        &FakeMembers(None),
    )
    .await
    .unwrap();
    assert_eq!(
        auth.view.token.models,
        vec![JsonData::String("user-model".to_owned())]
    );
}

#[tokio::test]
async fn preserved_session_fields() {
    let users = FakeUsers(UserAnswer::Found(Box::new(user_row(
        r#"{"user_id":"u1","teams":["t1"]}"#,
    ))));
    let teams = FakeTeams(Some(team_row(r#"{"team_id":"t1","team_alias":"Team"}"#)));
    let auth = refresh(&team_session_auth(), &users, &teams, &FakeMembers(None))
        .await
        .unwrap();
    assert_eq!(auth.view.token.key_name.as_deref(), Some("cli-session"));
    assert!(auth.extra.is_session_token);
    assert_eq!(auth.view.team_alias.as_deref(), Some("Team"));
    assert!(auth_text(&auth).contains(r#""token":"hashed-key""#));
}

#[tokio::test]
async fn missing_user_result_kept() {
    let auth = plain_session_auth();
    let refreshed = refresh(
        &auth,
        &FakeUsers(UserAnswer::Missing),
        &FakeTeams(None),
        &FakeMembers(None),
    )
    .await
    .unwrap();
    assert_eq!(auth_text(&refreshed), auth_text(&auth));
}

#[tokio::test]
async fn degraded_debug_keeps_grants() {
    let auth = plain_session_auth();
    let refreshed = refresh(
        &auth,
        &FakeUsers(UserAnswer::Unavailable),
        &FakeTeams(None),
        &FakeMembers(None),
    )
    .await
    .unwrap();
    assert_eq!(auth_text(&refreshed), auth_text(&auth));
}

#[tokio::test]
async fn deleted_user_refused() {
    let error = refresh(
        &plain_session_auth(),
        &FakeUsers(UserAnswer::Gone),
        &FakeTeams(None),
        &FakeMembers(None),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PublicDenial::UserGone(_)));
}

#[tokio::test]
async fn removed_member_refused() {
    let users = FakeUsers(UserAnswer::Found(Box::new(user_row(
        r#"{"user_id":"u1","teams":["other-team"]}"#,
    ))));
    let error = refresh(
        &team_session_auth(),
        &users,
        &FakeTeams(None),
        &FakeMembers(None),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        PublicDenial::NotAMember { team_id } if team_id == "t1"
    ));
}

#[tokio::test]
async fn missing_permission_input_skips() {
    let permissions = FakePermissions::new(Vec::new());
    let lookups = lookups(
        None,
        FakeCache::new(None),
        FakeWriter::new(false),
        Some(Arc::clone(&permissions)),
    );
    let found = resolve_object_permission_for_unresolvable_team(None, &lookups)
        .await
        .unwrap();
    assert!(found.is_none());
    assert!(permissions.calls().is_empty());
}

#[tokio::test]
async fn unreadable_team_permission_error() {
    let permissions = FakePermissions::new(vec![Err(PermissionError::NoDatabase)]);
    let lookups = lookups(
        None,
        FakeCache::new(None),
        FakeWriter::new(false),
        Some(permissions),
    );
    let error = resolve_object_permission_for_unresolvable_team(Some("op-1"), &lookups)
        .await
        .unwrap_err();
    assert!(matches!(error, PermissionError::NoDatabase));
}
