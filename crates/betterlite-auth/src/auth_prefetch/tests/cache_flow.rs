#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use tokio::time::{sleep, timeout};

use super::fixtures;
use crate::auth_prefetch::db::PrefetchDatabase;
use crate::auth_prefetch::entries::AuthObjectRefs;
use crate::auth_prefetch::fill::IdentityPrefetch;
use crate::auth_prefetch::orchestrator::{OwnerPrefetch, ReferencedPrefetch};
use crate::auth_prefetch::rows::RowKind;

use super::support::{
    Events, FakeCache, FakeDatabase, FakeRedis, FixedClock, HangingDatabase, Logs, ReadFailure,
    RecordingBinder, auth, events, json, recorded,
};

const MANAGEMENT_TTL: f64 = 60.0;
const ORGANIZATION_TTL: f64 = 7.0;

fn refs_user(user_id: &str) -> AuthObjectRefs {
    AuthObjectRefs {
        user_id: Some(user_id.to_owned()),
        team_id: None,
        membership_user_id: None,
        organization_id: None,
        project_id: None,
    }
}

fn refs_user_team() -> AuthObjectRefs {
    AuthObjectRefs {
        user_id: Some("user-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        membership_user_id: Some("user-1".to_owned()),
        organization_id: None,
        project_id: None,
    }
}

fn refs_all() -> AuthObjectRefs {
    AuthObjectRefs {
        organization_id: Some("org-1".to_owned()),
        project_id: Some("project-1".to_owned()),
        ..refs_user_team()
    }
}

fn owner<'context>(
    cache: &'context FakeCache,
    database: Option<&'context FakeDatabase>,
    clock: &'context FixedClock,
) -> OwnerPrefetch<'context> {
    OwnerPrefetch {
        cache,
        database: database.map(|database| -> &dyn PrefetchDatabase { database }),
        clock,
        organization_ttl: ORGANIZATION_TTL,
    }
}

fn member_rows() -> FakeDatabase {
    FakeDatabase::with(&[
        (RowKind::User, fixtures::user_row("user-1")),
        (RowKind::Team, fixtures::team_row("team-1")),
        (
            RowKind::Membership,
            fixtures::membership_row("user-1", "team-1"),
        ),
    ])
}

#[tokio::test]
async fn ordinary_partition_even_hex_user_id() {
    let hex = "a".repeat(64);
    let refs = refs_user(&hex);
    let clock = FixedClock { now: 10.0 };

    let cache = FakeCache::new(MANAGEMENT_TTL);
    cache
        .ordinary
        .seed(&hex, CacheValue::Text("seeded".to_owned()));
    cache
        .key_object
        .seed(&hex, CacheValue::Text("elsewhere".to_owned()));
    let database = FakeDatabase::with(&[(RowKind::User, fixtures::user_row(&hex))]);
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    assert!(database.calls().is_empty());
    assert_eq!(
        cache.ordinary.get_value(&hex),
        Some(CacheValue::Text("seeded".to_owned()))
    );
    assert!(cache.key_object.writes().is_empty());

    let key_only_cache = FakeCache::new(MANAGEMENT_TTL);
    key_only_cache
        .key_object
        .seed(&hex, CacheValue::Text("elsewhere".to_owned()));
    let key_only_database = FakeDatabase::with(&[(RowKind::User, fixtures::user_row(&hex))]);
    owner(&key_only_cache, Some(&key_only_database), &clock)
        .prefetch(&refs)
        .await;
    assert_eq!(key_only_database.calls(), vec![vec![RowKind::User]]);
    assert_eq!(key_only_cache.ordinary.write_keys(), vec![hex.clone()]);
    assert_eq!(
        key_only_cache.key_object.get_value(&hex),
        Some(CacheValue::Text("elsewhere".to_owned()))
    );
}

#[tokio::test]
async fn nonnull_memory_skips_prefetch() {
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::new(FakeRedis::batch()));
    cache.ordinary.seed("user-1", CacheValue::Boolean(false));
    cache
        .ordinary
        .seed("team_id:team-1", CacheValue::Text(String::new()));
    cache
        .ordinary
        .seed("team-1_user-1", CacheValue::Text("[]".to_owned()));
    cache
        .ordinary
        .seed("team_membership:user-1:team-1", CacheValue::Null);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    assert!(database.calls().is_empty());
    assert!(cache.redis.as_ref().expect("redis").reads().is_empty());
    assert!(cache.ordinary.writes().is_empty());
}

#[tokio::test]
async fn redis_sorted_missing_keys_with_duplicates() {
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::new(FakeRedis::batch()));
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    let refs = AuthObjectRefs {
        user_id: Some("team_id:t1".to_owned()),
        team_id: Some("t1".to_owned()),
        membership_user_id: Some("team_id:t1".to_owned()),
        organization_id: None,
        project_id: None,
    };
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    let reads = cache.redis.as_ref().expect("redis").reads();
    assert_eq!(
        reads,
        vec![vec![
            "t1_team_id:t1".to_owned(),
            "team_id:t1".to_owned(),
            "team_id:t1".to_owned(),
            "team_membership:team_id:t1:t1".to_owned(),
        ]]
    );
}

#[tokio::test]
async fn pipeline_failure_debug_then_db() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::batch());
    redis.fail_read(ReadFailure::Batch("the pipeline is down"));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&redis));
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert_eq!(database.calls(), vec![vec![RowKind::User]]);
    assert_eq!(cache.ordinary.write_keys(), vec!["user-1".to_owned()]);
    let text = logs.text();
    assert!(text.contains("DEBUG"), "{text}");
    assert!(
        text.contains("auth prefetch Redis read failed, filling from the database"),
        "{text}"
    );
}

#[tokio::test]
async fn direct_read_propagated_error_warns_without_db() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::default());
    redis.fail_read(ReadFailure::Direct("the direct read is down"));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert!(database.calls().is_empty());
    assert!(cache.ordinary.writes().is_empty());
    let text = logs.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(
        text.contains("auth prefetch skipped, falling back to per-object lookups"),
        "{text}"
    );
}

#[tokio::test]
async fn malformed_result_map_outer_warning() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::batch());
    redis.return_non_object();
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert!(database.calls().is_empty());
    assert!(cache.ordinary.writes().is_empty());
    assert!(
        logs.text()
            .contains("the Redis result is not a JSON object")
    );
}

#[tokio::test]
async fn redis_rows_cached_without_validation() {
    let redis = Arc::new(FakeRedis::batch());
    redis.seed("user-1", Json::Text("garbage".to_owned()));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert!(database.calls().is_empty());
    let writes = cache.ordinary.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key, "user-1");
    assert_eq!(writes[0].value, CacheValue::Text("garbage".to_owned()));
    assert_eq!(writes[0].ttl, Some(MANAGEMENT_TTL));
}

#[tokio::test]
async fn redis_null_stays_missing() {
    let redis = Arc::new(FakeRedis::batch());
    redis.seed("user-1", Json::Null);
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database = FakeDatabase::with(&[(RowKind::User, fixtures::user_row("user-1"))]);
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert_eq!(database.calls(), vec![vec![RowKind::User]]);
    let writes = cache.ordinary.writes();
    assert_eq!(writes.len(), 1);
    assert!(matches!(writes[0].value, CacheValue::Object(_)));
}

#[tokio::test]
async fn ttl_none_omits_memory_argument() {
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    let writes = cache.ordinary.writes();
    assert_eq!(
        writes
            .iter()
            .map(|write| (write.key.as_str(), write.ttl))
            .collect::<Vec<_>>(),
        vec![
            ("user-1", Some(MANAGEMENT_TTL)),
            ("team_id:team-1", Some(MANAGEMENT_TTL)),
            ("team-1_user-1", Some(5.0)),
            ("team_membership:user-1:team-1", None),
        ]
    );
}

#[tokio::test]
async fn only_original_misses_rechecked() {
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::new(FakeRedis::batch()));
    cache
        .ordinary
        .seed("user-1", CacheValue::Text("already there".to_owned()));
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    assert_eq!(
        database.calls(),
        vec![vec![RowKind::Team, RowKind::Membership]]
    );
    assert_eq!(
        cache.ordinary.get_value("user-1"),
        Some(CacheValue::Text("already there".to_owned()))
    );
    assert_eq!(
        cache.ordinary.write_keys(),
        vec![
            "team_id:team-1".to_owned(),
            "team-1_user-1".to_owned(),
            "team_membership:user-1:team-1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn no_db_after_redis_fill() {
    let redis = Arc::new(FakeRedis::batch());
    redis.seed("user-1", Json::Text("from redis".to_owned()));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert!(database.calls().is_empty());
    assert_eq!(cache.ordinary.write_keys(), vec!["user-1".to_owned()]);

    let cold_cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::new(FakeRedis::batch()));
    owner(&cold_cache, None, &clock)
        .prefetch(&refs_user("user-1"))
        .await;
    assert!(cold_cache.ordinary.writes().is_empty());
}

#[tokio::test]
async fn row_kind_validated_once() {
    let (logs, _guard) = Logs::capture();
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let database = FakeDatabase::with(&[(RowKind::Organization, json(r#""not a row""#))]);
    let clock = FixedClock { now: 10.0 };
    let refs = AuthObjectRefs {
        user_id: None,
        team_id: None,
        membership_user_id: None,
        organization_id: Some("org-1".to_owned()),
        project_id: None,
    };
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    assert!(cache.ordinary.writes().is_empty());
    let text = logs.text();
    assert_eq!(
        text.matches("auth prefetch: the row did not validate as its model")
            .count(),
        1,
        "{text}"
    );

    let valid_cache = FakeCache::new(MANAGEMENT_TTL);
    let valid_database =
        FakeDatabase::with(&[(RowKind::Organization, fixtures::organization_row("org-1"))]);
    owner(&valid_cache, Some(&valid_database), &clock)
        .prefetch(&refs)
        .await;
    let writes = valid_cache.ordinary.writes();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].value, writes[1].value);
    assert_eq!(
        valid_cache.ordinary.write_keys(),
        vec![
            "org_id:org-1".to_owned(),
            "org_id:org-1:with_budget".to_owned()
        ]
    );
}

#[tokio::test]
async fn missing_row_not_negative_cached() {
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::new(FakeRedis::batch()));
    let database = FakeDatabase::default();
    let clock = FixedClock { now: 10.0 };
    let refs = refs_user("user-1");
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    assert_eq!(
        database.calls(),
        vec![vec![RowKind::User], vec![RowKind::User]]
    );
    assert!(cache.ordinary.writes().is_empty());
    assert!(cache.redis.as_ref().expect("redis").queued().is_empty());
    assert!(cache.ordinary.get_value("user-1").is_none());
}

#[tokio::test]
async fn serialization_all_before_memory() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::default());
    redis.fail_writes();
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let database =
        FakeDatabase::with(&[(RowKind::Organization, fixtures::organization_row("org-1"))]);
    let clock = FixedClock { now: 10.0 };
    let refs = AuthObjectRefs {
        user_id: None,
        team_id: None,
        membership_user_id: None,
        organization_id: Some("org-1".to_owned()),
        project_id: None,
    };
    owner(&cache, Some(&database), &clock).prefetch(&refs).await;
    let writes = cache.ordinary.writes();
    assert_eq!(
        writes
            .iter()
            .map(|write| write.key.as_str())
            .collect::<Vec<_>>(),
        vec!["org_id:org-1", "org_id:org-1:with_budget"]
    );
    let pipelines = cache.redis.as_ref().expect("redis").pipelines();
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].len(), 2);
    assert!(logs.text().contains("the Redis pipeline write failed"));
}

#[tokio::test]
async fn memory_writes_entry_order() {
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let database = FakeDatabase::with(&[
        (RowKind::User, fixtures::user_row("user-1")),
        (RowKind::Team, fixtures::team_row("team-1")),
        (
            RowKind::Membership,
            fixtures::membership_row("user-1", "team-1"),
        ),
        (RowKind::Organization, fixtures::organization_row("org-1")),
        (RowKind::Project, fixtures::project_row("project-1")),
    ]);
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_all())
        .await;
    assert_eq!(
        cache.ordinary.write_keys(),
        vec![
            "user-1".to_owned(),
            "team_id:team-1".to_owned(),
            "team-1_user-1".to_owned(),
            "team_membership:user-1:team-1".to_owned(),
            "org_id:org-1".to_owned(),
            "org_id:org-1:with_budget".to_owned(),
            "project_id:project-1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn db_membership_memory_uses_default_ttl() {
    let mut cache = FakeCache::new(MANAGEMENT_TTL);
    cache.default_memory_ttl = Some(600.0);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    let ttls: Vec<Option<f64>> = cache
        .ordinary
        .writes()
        .into_iter()
        .map(|write| write.ttl)
        .collect();
    assert_eq!(
        ttls,
        vec![
            Some(MANAGEMENT_TTL),
            Some(MANAGEMENT_TTL),
            Some(5.0),
            Some(600.0),
        ]
    );
}

#[tokio::test]
async fn redis_retains_original_none_ttl() {
    let redis = Arc::new(FakeRedis::batch());
    let mut cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&redis));
    cache.default_memory_ttl = Some(600.0);
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    let queued: Vec<(String, Option<f64>)> = redis
        .queued()
        .into_iter()
        .map(|write| (write.key, write.ttl))
        .collect();
    assert_eq!(
        queued,
        vec![
            ("user-1".to_owned(), Some(MANAGEMENT_TTL)),
            ("team_id:team-1".to_owned(), Some(MANAGEMENT_TTL)),
            ("team-1_user-1".to_owned(), Some(5.0)),
            (
                "team_membership:user-1:team-1".to_owned(),
                Option::<f64>::None
            ),
        ]
    );

    let pipeline_redis = Arc::new(FakeRedis::default());
    let pipeline_cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&pipeline_redis));
    let pipeline_database = member_rows();
    owner(&pipeline_cache, Some(&pipeline_database), &clock)
        .prefetch(&refs_user_team())
        .await;
    let pipelines = pipeline_redis.pipelines();
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].len(), 4);
    assert_eq!(pipelines[0][2].ttl, Some(5.0));
    assert_eq!(pipelines[0][3].ttl, None);
}

#[tokio::test]
async fn writeback_pipeline_queues_without_await() {
    let redis = Arc::new(FakeRedis::batch());
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&redis));
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    let queued = redis.queued();
    assert_eq!(queued.len(), 4);
    assert_eq!(queued[0].key, "user-1");
    assert_eq!(queued[3].key, "team_membership:user-1:team-1");
    assert!(redis.pipelines().is_empty());
}

#[tokio::test]
async fn later_failure_preserves_earlier_writes() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::batch());
    redis.fail_queue_for("team-1_user-1");
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&redis));
    let database = member_rows();
    let clock = FixedClock { now: 10.0 };
    owner(&cache, Some(&database), &clock)
        .prefetch(&refs_user_team())
        .await;
    assert_eq!(
        cache.ordinary.write_keys(),
        vec![
            "user-1".to_owned(),
            "team_id:team-1".to_owned(),
            "team-1_user-1".to_owned(),
            "team_membership:user-1:team-1".to_owned(),
        ]
    );
    assert_eq!(
        redis
            .queued()
            .into_iter()
            .map(|write| write.key)
            .collect::<Vec<_>>(),
        vec!["user-1".to_owned(), "team_id:team-1".to_owned()]
    );
    assert!(
        logs.text()
            .contains("auth prefetch skipped, falling back to per-object lookups")
    );
}

#[tokio::test]
async fn owner_prefetch_cancellation_propagates() {
    let (logs, _guard) = Logs::capture();
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let dropped = Arc::new(AtomicBool::new(false));
    let database = HangingDatabase {
        dropped: Arc::clone(&dropped),
    };
    let clock = FixedClock { now: 10.0 };
    let prefetch = OwnerPrefetch {
        cache: &cache,
        database: Some(&database),
        clock: &clock,
        organization_ttl: ORGANIZATION_TTL,
    };
    let cancelled = timeout(
        Duration::from_millis(50),
        prefetch.prefetch(&refs_user("user-1")),
    )
    .await;
    assert!(cancelled.is_err());
    timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the database future was dropped");
    assert!(!logs.text().contains("auth prefetch skipped"));
}

#[tokio::test]
async fn identity_no_redis_noop() {
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let prefetch = IdentityPrefetch { cache: &cache };
    prefetch
        .prefetch(&["model_access_group_registry".to_owned()])
        .await
        .expect("the prefetch without redis succeeds");
    assert!(cache.ordinary.reads().is_empty());
    assert!(cache.key_object.reads().is_empty());
    assert!(cache.ordinary.writes().is_empty());
}

#[tokio::test]
async fn identity_deduplicates_first_seen() {
    let redis = Arc::new(FakeRedis::batch());
    redis.seed("b", Json::Text("registry".to_owned()));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, Arc::clone(&redis));
    let prefetch = IdentityPrefetch { cache: &cache };
    prefetch
        .prefetch(&["b".to_owned(), "a".to_owned(), "b".to_owned()])
        .await
        .expect("the identity prefetch succeeds");
    assert_eq!(redis.reads(), vec![vec!["a".to_owned(), "b".to_owned()]]);
    assert_eq!(cache.ordinary.write_keys(), vec!["b".to_owned()]);
}

#[tokio::test]
async fn identity_uses_selected_partition() {
    let hex = "b".repeat(64);
    let end_user = "end_user_id:u".to_owned();
    let redis = Arc::new(FakeRedis::batch());
    redis.seed(&hex, Json::Text("registry".to_owned()));
    redis.seed(&end_user, Json::Text("registry".to_owned()));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let prefetch = IdentityPrefetch { cache: &cache };
    prefetch
        .prefetch(&[hex.clone(), end_user.clone()])
        .await
        .expect("the identity prefetch succeeds");
    assert_eq!(
        cache.key_object.get_value(&hex),
        Some(CacheValue::Text("registry".to_owned()))
    );
    assert_eq!(
        cache.ordinary.get_value(&end_user),
        Some(CacheValue::Text("registry".to_owned()))
    );
    assert_eq!(cache.key_object.writes()[0].ttl, Some(30.0));
    assert_eq!(cache.ordinary.writes()[0].ttl, Some(30.0));
}

#[tokio::test]
async fn identity_read_failure_warns_and_returns() {
    let (logs, _guard) = Logs::capture();
    let redis = Arc::new(FakeRedis::default());
    redis.fail_read(ReadFailure::Direct("the direct read is down"));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    let prefetch = IdentityPrefetch { cache: &cache };
    prefetch
        .prefetch(&["model_access_group_registry".to_owned()])
        .await
        .expect("the read failure is best effort");
    assert!(cache.ordinary.writes().is_empty());
    let text = logs.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(
        text.contains("auth identity prefetch skipped, falling back to per-key lookups"),
        "{text}"
    );
}

#[tokio::test]
async fn identity_memory_write_failure_propagates() {
    let redis = Arc::new(FakeRedis::batch());
    redis.seed("end_user_id:u", Json::Text("registry".to_owned()));
    let cache = FakeCache::with_redis(MANAGEMENT_TTL, redis);
    cache.ordinary.fail_writes_for("end_user_id:u");
    let prefetch = IdentityPrefetch { cache: &cache };
    let outcome = prefetch.prefetch(&["end_user_id:u".to_owned()]).await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn counter_binding_precedes_owner_prefetch() {
    let events: Events = events();
    let binder = RecordingBinder::new(&events);
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let mut database = FakeDatabase::default();
    database.record_events(Arc::clone(&events));
    let clock = FixedClock { now: 10.0 };
    let referenced = ReferencedPrefetch {
        owner: owner(&cache, Some(&database), &clock),
        admission: &binder,
    };
    referenced
        .prefetch(&auth(&[("user_id", json(r#""user-1""#))]), None)
        .await;
    assert_eq!(recorded(&events), vec!["bind".to_owned(), "db".to_owned()]);
}

#[tokio::test]
async fn falsey_end_user_normalized_for_binding() {
    let events: Events = events();
    let binder = RecordingBinder::new(&events);
    let cache = FakeCache::new(MANAGEMENT_TTL);
    let database = FakeDatabase::default();
    let clock = FixedClock { now: 10.0 };
    let referenced = ReferencedPrefetch {
        owner: owner(&cache, Some(&database), &clock),
        admission: &binder,
    };
    let api_key = auth(&[("user_id", json(r#""user-1""#))]);
    referenced.prefetch(&api_key, Some("")).await;
    referenced.prefetch(&api_key, Some("user-9")).await;
    referenced.prefetch(&api_key, None).await;
    assert_eq!(
        binder.bindings(),
        vec![None, Some("user-9".to_owned()), None]
    );
}
