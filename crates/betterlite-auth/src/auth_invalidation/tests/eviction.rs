use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use super::support::{self, Call, Fault, Recorder};
use crate::auth_invalidation::AuthInvalidation;

fn deletion(recorder: &Arc<Recorder>, usage: Option<&Arc<Recorder>>) -> AuthInvalidation {
    support::builder(recorder, usage)
}

#[test]
fn team_id_before_alias() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&usage));
    runtime.block_on(invalidation.delete_cache_team_object("team-1", Some("alias-1")));
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthDeleteSync("team_id:team-1".to_owned()),
            Call::Publish {
                key: "team_id:team-1".to_owned(),
                new_value: None,
                ttl: None,
            },
            Call::AuthDeleteSync("team_alias:alias-1".to_owned()),
            Call::Publish {
                key: "team_alias:alias-1".to_owned(),
                new_value: None,
                ttl: None,
            },
        ]
    );
    assert_eq!(
        usage.calls(),
        vec![
            Call::UsageDeleteAsync("team_id:team-1".to_owned()),
            Call::UsageDeleteAsync("team_alias:alias-1".to_owned()),
        ]
    );
}

#[test]
fn local_auth_failure_skips_usage_but_publishes() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::AuthDeleteSync);
    let usage = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&usage));
    let ((), warnings) = support::capture(|| {
        runtime.block_on(invalidation.delete_cache_team_object("team-1", Some("alias-1")))
    });
    assert!(
        usage.calls().is_empty(),
        "an auth delete failure skips usage deletion for that key"
    );
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthDeleteSync("team_id:team-1".to_owned()),
            Call::Publish {
                key: "team_id:team-1".to_owned(),
                new_value: None,
                ttl: None,
            },
            Call::AuthDeleteSync("team_alias:alias-1".to_owned()),
            Call::Publish {
                key: "team_alias:alias-1".to_owned(),
                new_value: None,
                ttl: None,
            },
        ],
        "each key is published even when its auth delete fails"
    );
    assert_eq!(
        support::warned_subjects(&warnings),
        vec!["team_id:team-1".to_owned(), "team_alias:alias-1".to_owned()]
    );
}

#[test]
fn usage_failure_warns_then_publishes() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    usage.fail(Fault::UsageDeleteAsync);
    let invalidation = deletion(&recorder, Some(&usage));
    let ((), warnings) = support::capture(|| {
        runtime.block_on(invalidation.delete_cache_team_object("team-1", Some("alias-1")))
    });
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthDeleteSync("team_id:team-1".to_owned()),
            Call::Publish {
                key: "team_id:team-1".to_owned(),
                new_value: None,
                ttl: None,
            },
            Call::AuthDeleteSync("team_alias:alias-1".to_owned()),
            Call::Publish {
                key: "team_alias:alias-1".to_owned(),
                new_value: None,
                ttl: None,
            },
        ]
    );
    assert_eq!(
        support::warned_subjects(&warnings),
        vec!["team_id:team-1".to_owned(), "team_alias:alias-1".to_owned()]
    );
}

#[test]
fn missing_usage_cache_still_publishes() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = deletion(&recorder, None);
    runtime.block_on(invalidation.delete_cache_team_object("team-1", None));
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthDeleteSync("team_id:team-1".to_owned()),
            Call::Publish {
                key: "team_id:team-1".to_owned(),
                new_value: None,
                ttl: None,
            },
        ]
    );
}

#[test]
fn key_delete_local_usage_publish_order() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&usage));
    let ((), warnings) =
        support::capture(|| runtime.block_on(invalidation.delete_cache_key_object("hash-1")));
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthDeleteSync("hash-1".to_owned()),
            Call::Publish {
                key: "hash-1".to_owned(),
                new_value: None,
                ttl: None,
            },
        ]
    );
    assert_eq!(
        usage.calls(),
        vec![Call::UsageDeleteAsync("hash-1".to_owned())]
    );
    assert!(warnings.is_empty());
}

#[test]
fn batch_keys_concurrent_with_duplicates() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&usage));
    let tokens = vec![
        "hash-1".to_owned(),
        "hash-1".to_owned(),
        "hash-2".to_owned(),
    ];
    runtime.block_on(invalidation.delete_cache_key_objects(&tokens));
    assert_eq!(
        support::call_keys(&recorder.calls()),
        vec!["hash-1", "hash-1", "hash-1", "hash-1", "hash-2", "hash-2"],
        "one local delete and one broadcast per input, duplicates kept"
    );
    assert_eq!(
        usage.calls().len(),
        3,
        "every input reaches the usage cache"
    );
}

#[test]
fn batch_child_errors_warn_in_input_order() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::AuthDeleteSync);
    let invalidation = deletion(&recorder, None);
    let tokens = vec!["b-token".to_owned(), "a-token".to_owned()];
    let ((), warnings) =
        support::capture(|| runtime.block_on(invalidation.delete_cache_key_objects(&tokens)));
    assert_eq!(
        support::warned_subjects(&warnings),
        vec!["b-token".to_owned(), "a-token".to_owned()],
        "warnings follow the caller's input order, not completion order"
    );
}

#[test]
fn batch_empty_noop() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&recorder));
    runtime.block_on(invalidation.delete_cache_key_objects(&[]));
    assert!(recorder.calls().is_empty());
}

#[test]
fn outer_cancellation_propagates() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::UsagePending);
    let invalidation = deletion(&recorder, Some(&recorder));
    let tokens = vec!["hash-1".to_owned()];
    let outcome = runtime.block_on(async {
        timeout(
            Duration::from_millis(50),
            invalidation.delete_cache_key_objects(&tokens),
        )
        .await
    });
    let _elapsed = outcome
        .expect_err("cancelling the batch drops the pending child instead of waiting it out");
}

#[test]
fn access_delete_failure_propagates_without_publish() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::AuthDeleteSync);
    let usage = Recorder::new(None);
    let invalidation = deletion(&recorder, Some(&usage));
    let outcome = runtime.block_on(invalidation.delete_cache_access_object("ag-1"));
    outcome.unwrap_err();
    assert_eq!(
        recorder.calls(),
        vec![Call::AuthDeleteSync("access_group_id:ag-1".to_owned())]
    );
    assert!(usage.calls().is_empty());
}

#[test]
fn project_key_delegation() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = deletion(&recorder, None);
    runtime.block_on(invalidation.delete_cached_project_object("project-1"));
    assert_eq!(
        recorder.calls(),
        vec![Call::Evict {
            keys: vec!["project_id:project-1".to_owned()],
        }]
    );
}

#[test]
fn project_eviction_uses_pubsub_best_effort_port() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::ApplyLocally);
    recorder.fail(Fault::AuthDeleteRedis);
    let invalidation = deletion(&recorder, None);
    runtime.block_on(invalidation.delete_cached_project_object("project-1"));
    assert_eq!(
        recorder.calls(),
        vec![
            Call::Evict {
                keys: vec!["project_id:project-1".to_owned()],
            },
            Call::AuthDeleteMemory("project_id:project-1".to_owned()),
            Call::AuthDeleteRedis("project_id:project-1".to_owned()),
        ],
        "local eviction goes through the transport's best-effort apply"
    );
}

#[test]
fn self_delivery_keeps_new_values() {
    use crate::auth_pubsub::eviction::LocalEviction as _;
    use crate::auth_pubsub::message::Invalidation;

    let recorder = Recorder::new(None);
    let invalidation = deletion(&recorder, None);
    let outcome = invalidation.apply(Invalidation {
        cache_key: "team_membership:user-1:team-1".to_owned(),
        new_value: Some(3.0),
        ttl: Some(60.0),
    });
    outcome.unwrap();
    assert!(
        recorder.calls().is_empty(),
        "a self-delivered SET writes into the additional caches, never erasing the local value"
    );
}
