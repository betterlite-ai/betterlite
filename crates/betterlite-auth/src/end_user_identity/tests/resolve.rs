#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::super::cache::{NEGATIVE_TTL_SECONDS, POSITIVE_TTL_SECONDS, ValidationCache};
use super::super::existence::UserExistence;
use super::super::resolve::{DatabaseLookups, EndUserIdResolver, ResolveError, ResolveRequest};
use super::support::{
    Answer, ScriptedCache, ScriptedEndUsers, ScriptedUserStore, ScriptedUsers, UserCachePort,
    assert_wrote, capture_events, debug_messages, resolver_for, user_lookup,
};
use crate::auth_cache::WriteTtl;
use crate::auth_user_lookup::lookup::UserLookup;

fn collaborators(end_users: Answer, users: Answer) -> (Arc<ScriptedEndUsers>, Arc<ScriptedUsers>) {
    (
        Arc::new(ScriptedEndUsers::new(end_users)),
        Arc::new(ScriptedUsers::new(users)),
    )
}

const fn request<'request>(
    raw_end_user_id: Option<&'request str>,
    key_end_user_budget_id: Option<&'request str>,
) -> ResolveRequest<'request> {
    ResolveRequest {
        raw_end_user_id,
        key_end_user_budget_id,
    }
}

#[tokio::test]
async fn unknown_enforced_user() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, Some("proxy"));
    let resolved = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert_eq!(resolved.as_deref(), Some("ghost"));
    assert_eq!(end_users.calls(), vec!["ghost".to_owned()]);
    let probes = users.probes();
    assert_eq!(probes.len(), 1);
    assert_eq!(probes[0].user_id, "ghost");
    assert!(probes[0].user_email.is_none());
    assert!(!probes[0].user_id_upsert);
    assert!(!probes[0].check_db_only);
    assert_wrote(&cache, "ghost", "invalid", NEGATIVE_TTL_SECONDS);
}

#[tokio::test]
async fn validation_off_no_db_passthrough() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let flag_off = resolver_for(Arc::clone(&cache), &end_users, &users, false, None);
    let off = flag_off
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the passthrough");
    assert_eq!(off.as_deref(), Some("ghost"));
    assert!(cache.reads().is_empty());
    assert!(end_users.calls().is_empty());
    assert!(users.probes().is_empty());
    let validation: Arc<dyn ValidationCache> = Arc::clone(&cache);
    let without_database = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: None,
        cache: validation,
    };
    let no_db = without_database
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the passthrough");
    assert_eq!(no_db.as_deref(), Some("ghost"));
    let absent = flag_off
        .resolve(&ResolveRequest::default())
        .await
        .expect("the absent id");
    assert!(absent.is_none());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn positive_cache_300() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Found, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let first = identity
        .resolve(&request(Some("alice"), None))
        .await
        .expect("the lookup");
    assert_eq!(first.as_deref(), Some("alice"));
    assert_wrote(&cache, "alice", "valid", POSITIVE_TTL_SECONDS);
    let second = identity
        .resolve(&request(Some("alice"), None))
        .await
        .expect("the cached lookup");
    assert_eq!(second.as_deref(), Some("alice"));
    assert_eq!(end_users.calls(), vec!["alice".to_owned()]);
    assert_eq!(cache.writes().len(), 1);
}

#[tokio::test]
async fn negative_cache_60() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let first = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert!(first.is_none());
    assert_wrote(&cache, "ghost", "invalid", NEGATIVE_TTL_SECONDS);
    let second = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the cached lookup");
    assert!(second.is_none());
    assert_eq!(end_users.calls(), vec!["ghost".to_owned()]);
    assert_eq!(users.probes().len(), 1);
    assert_eq!(cache.reads().len(), 2);
}

#[tokio::test]
async fn unknown_cache_value_miss() {
    let cache = Arc::new(ScriptedCache::new());
    let values: Vec<Json> =
        serde_json::from_str(r#"[null,true,7,"maybe",[],{}]"#).expect("the cached values");
    let (end_users, users) = collaborators(Answer::Found, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let mut ids = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let id = format!("ghost-{index}");
        cache.seed(&id, value.clone());
        ids.push(id);
    }
    for id in &ids {
        let resolved = identity
            .resolve(&request(Some(id), None))
            .await
            .expect("the lookup");
        assert_eq!(resolved.as_deref(), Some(id.as_str()));
    }
    assert_eq!(end_users.calls(), ids);
    let writes = cache.writes();
    assert_eq!(writes.len(), values.len());
    for write in &writes {
        assert_eq!(write.value, Json::Text("valid".to_owned()));
        assert_eq!(
            write.ttl,
            WriteTtl {
                seconds: POSITIVE_TTL_SECONDS
            }
        );
    }
}

#[tokio::test]
async fn key_empty_budget_preserves_id() {
    let cache = Arc::new(ScriptedCache::new());
    cache.seed("ghost", Json::Text("invalid".to_owned()));
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let preserved = identity
        .resolve(&request(Some("ghost"), Some("")))
        .await
        .expect("the lookup");
    assert_eq!(preserved.as_deref(), Some("ghost"));
    assert!(end_users.calls().is_empty());
    assert!(cache.writes().is_empty());
    let fresh = identity
        .resolve(&request(Some("ghost-2"), Some("")))
        .await
        .expect("the lookup");
    assert_eq!(fresh.as_deref(), Some("ghost-2"));
    assert_wrote(&cache, "ghost-2", "invalid", NEGATIVE_TTL_SECONDS);
}

#[tokio::test]
async fn global_empty_budget_does_not() {
    let cache = Arc::new(ScriptedCache::new());
    cache.seed("ghost", Json::Text("invalid".to_owned()));
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, Some(""));
    let dropped = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert!(dropped.is_none());
    assert!(end_users.calls().is_empty());
    let fresh = identity
        .resolve(&request(Some("ghost-2"), None))
        .await
        .expect("the lookup");
    assert!(fresh.is_none());
    assert_wrote(&cache, "ghost-2", "invalid", NEGATIVE_TTL_SECONDS);
}

#[tokio::test]
async fn end_user_before_user() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Found, Answer::Error);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let found = identity
        .resolve(&request(Some("alice"), None))
        .await
        .expect("the lookup");
    assert_eq!(found.as_deref(), Some("alice"));
    assert!(users.probes().is_empty());
    assert_wrote(&cache, "alice", "valid", POSITIVE_TTL_SECONDS);
}

#[tokio::test]
async fn end_user_lookup_failure_falls_back_to_the_user() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Error, Answer::Found);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let found = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert_eq!(found.as_deref(), Some("ghost"));
    assert_eq!(end_users.calls(), vec!["ghost".to_owned()]);
    assert_eq!(users.probes().len(), 1);
}

#[tokio::test]
async fn email_only_with_at() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Missing, Answer::Found);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let by_email = identity
        .resolve(&request(Some("alice@example.com"), None))
        .await
        .expect("the lookup");
    assert_eq!(by_email.as_deref(), Some("alice@example.com"));
    let by_name = identity
        .resolve(&request(Some("alice"), None))
        .await
        .expect("the lookup");
    assert_eq!(by_name.as_deref(), Some("alice"));
    let probes = users.probes();
    assert_eq!(probes.len(), 2);
    assert_eq!(probes[0].user_email.as_deref(), Some("alice@example.com"));
    assert!(probes[1].user_email.is_none());
}

#[tokio::test]
async fn no_user_upsert() {
    let store = Arc::new(ScriptedUserStore::default());
    let user_cache = Arc::new(UserCachePort::default());
    let lookup = Arc::new(user_lookup(&user_cache, &store));
    let users: Arc<dyn UserExistence> = Arc::<UserLookup>::clone(&lookup);
    let end_users = Arc::new(ScriptedEndUsers::new(Answer::Missing));
    let cache = Arc::new(ScriptedCache::new());
    let validation: Arc<dyn ValidationCache> = Arc::clone(&cache);
    let identity = EndUserIdResolver {
        validate_end_user_id_in_db: true,
        max_end_user_budget_id: None,
        lookups: Some(DatabaseLookups {
            end_users: Arc::<ScriptedEndUsers>::clone(&end_users),
            users,
        }),
        cache: validation,
    };
    let resolved = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert!(resolved.is_none());
    let calls = store.calls();
    assert!(calls.contains(&"find_by_id"));
    assert!(!calls.contains(&"create"));
    lookup.tasks.close();
    lookup.tasks.wait().await;
}

#[tokio::test]
async fn lookup_errors_debug_then_negative() {
    let (log, _guard) = capture_events();
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Error, Answer::Error);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let resolved = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("the lookup");
    assert!(resolved.is_none());
    let messages = debug_messages(&log);
    assert_eq!(messages.len(), 2);
    assert!(messages[0].starts_with("end_user validation: get_end_user_object lookup failed:"));
    assert!(messages[1].starts_with("end_user validation: get_user_object lookup failed:"));
    assert_wrote(&cache, "ghost", "invalid", NEGATIVE_TTL_SECONDS);
}

#[tokio::test]
async fn cache_failures_propagate() {
    let (end_users, users) = collaborators(Answer::Found, Answer::Missing);
    let read_failure = Arc::new(ScriptedCache::failing_reads());
    let reading = resolver_for(Arc::clone(&read_failure), &end_users, &users, true, None);
    let error = reading
        .resolve(&request(Some("alice"), None))
        .await
        .expect_err("the read failure");
    assert!(matches!(error, ResolveError::CacheRead(_)));
    assert!(end_users.calls().is_empty());
    let write_failure = Arc::new(ScriptedCache::failing_writes());
    let writing = resolver_for(Arc::clone(&write_failure), &end_users, &users, true, None);
    let error = writing
        .resolve(&request(Some("alice"), None))
        .await
        .expect_err("the write failure");
    assert!(matches!(error, ResolveError::CacheWrite(_)));
    assert_wrote(&write_failure, "alice", "valid", POSITIVE_TTL_SECONDS);
    assert_eq!(read_failure.reads().len(), 1);
}

#[tokio::test]
async fn unknown_id_dropped_not_denied() {
    let cache = Arc::new(ScriptedCache::new());
    let (end_users, users) = collaborators(Answer::Missing, Answer::Missing);
    let identity = resolver_for(Arc::clone(&cache), &end_users, &users, true, None);
    let dropped = identity
        .resolve(&request(Some("ghost"), None))
        .await
        .expect("an unknown id is not an error");
    assert!(dropped.is_none());
    assert_wrote(&cache, "ghost", "invalid", NEGATIVE_TTL_SECONDS);
}
