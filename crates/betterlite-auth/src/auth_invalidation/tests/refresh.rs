use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{self, Call, FIXED_NOW, Fault, MANAGEMENT_TTL, Recorder};
use crate::auth_cache::payload;
use crate::auth_invalidation::AuthInvalidation;

fn refresh(recorder: &Arc<Recorder>, usage: Option<&Arc<Recorder>>) -> AuthInvalidation {
    support::builder(recorder, usage)
}

#[test]
fn management_model_type_and_ttl() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let team = support::team("team-1", None);
    let expected = support::encoded_team(&team);
    runtime
        .block_on(invalidation.cache_management_object("team_id:team-1", &team))
        .expect("the management write succeeds");
    assert_eq!(
        recorder.calls(),
        vec![Call::AuthWrite {
            key: "team_id:team-1".to_owned(),
            payload: expected,
            ttl: Some(MANAGEMENT_TTL),
        }]
    );
}

#[test]
fn management_logging_argument_unused() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let invalidation = refresh(&recorder, Some(&recorder));
    let team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_management_object("team_id:team-1", &team))
        .expect("the management write succeeds");
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Publish { .. } | Call::Evict { .. })),
        "a management write touches no cache other than its own"
    );
}

#[test]
fn team_id_and_alias_keys() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthWrite {
                key: "team_id:team-1".to_owned(),
                payload: support::encoded_team(&team),
                ttl: Some(MANAGEMENT_TTL),
            },
            Call::AuthPreCallDelete("team_alias:alias-1".to_owned()),
        ]
    );
    assert_eq!(
        usage.calls(),
        vec![
            Call::UsageDeleteMemory("team_id:team-1".to_owned()),
            Call::UsageDeleteMemory("team_alias:alias-1".to_owned()),
        ]
    );
}

#[test]
fn team_timestamp_mutates_input() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", None);
    assert_eq!(team.last_refreshed_at, None);
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert_eq!(team.last_refreshed_at, Some(FIXED_NOW));
    let Call::AuthWrite { payload, .. } = &recorder.calls()[0] else {
        panic!("the refresh writes the team");
    };
    assert_eq!(
        support::number(payload, "last_refreshed_at"),
        Some(FIXED_NOW)
    );
    assert_eq!(
        support::field(payload, "team_id"),
        Some(&Json::Text("team-1".to_owned()))
    );
}

#[test]
fn access_group_key_and_independent_ttl() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let table = support::access_group("ag-1", "Group One");
    runtime
        .block_on(invalidation.cache_access_object("ag-1", &table))
        .expect("the access group write succeeds");
    let team = support::team("team-1", None);
    runtime
        .block_on(invalidation.cache_management_object("team_id:team-1", &team))
        .expect("the management write succeeds");
    let calls = recorder.calls();
    assert_eq!(
        calls,
        vec![
            Call::AuthWrite {
                key: "access_group_id:ag-1".to_owned(),
                payload: payload::encode(&table).expect("the table encodes"),
                ttl: Some(support::ACCESS_TTL),
            },
            Call::AuthWrite {
                key: "team_id:team-1".to_owned(),
                payload: support::encoded_team(&team),
                ttl: Some(MANAGEMENT_TTL),
            },
        ]
    );
}

#[test]
fn management_ttl_from_cache_settings() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.set_management_ttl(90.0);
    let invalidation = refresh(&recorder, None);
    let team = support::team("team-1", None);
    runtime
        .block_on(invalidation.cache_management_object("team_id:team-1", &team))
        .expect("the management write succeeds");
    let Call::AuthWrite { ttl, .. } = &recorder.calls()[0] else {
        panic!("the refresh writes the team");
    };
    assert_eq!(*ttl, Some(90.0_f64));
}

#[test]
fn key_timestamp_and_request_field_copy() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut auth = support::key_auth("/v1/chat/completions");
    runtime
        .block_on(invalidation.cache_key_object("hashed-key-1", &mut auth))
        .expect("the key refresh succeeds");
    assert_eq!(auth.view.last_refreshed_at, Some(Decimal::new(10_005, 1)));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    let Call::AuthWrite { key, payload, ttl } = &calls[0] else {
        panic!("the refresh writes the key");
    };
    assert_eq!(key, "hashed-key-1");
    assert_eq!(*ttl, Some(MANAGEMENT_TTL));
    assert_eq!(
        support::field(payload, "request_route"),
        Some(&Json::Null),
        "the cached copy clears request_route"
    );
    assert_eq!(
        support::number(payload, "last_refreshed_at"),
        Some(FIXED_NOW)
    );
}

#[test]
fn key_hash_not_recomputed() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut auth = support::key_auth("/v1/chat/completions");
    runtime
        .block_on(invalidation.cache_key_object("hashed-key-1", &mut auth))
        .expect("the key refresh succeeds");
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    let Call::AuthWrite { key, payload, .. } = &calls[0] else {
        panic!("the refresh writes the key");
    };
    assert_eq!(key, "hashed-key-1");
    assert_eq!(
        support::field(payload, "token"),
        Some(&Json::Text("sk-uncached".to_owned())),
        "the stored token is the caller's, not a hash of the cache key"
    );
}

#[test]
fn key_copy_retains_original_request_fields() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut auth = support::key_auth("/v1/chat/completions");
    assert!(auth.extra.budget_reservation.is_some());
    runtime
        .block_on(invalidation.cache_key_object("hashed-key-1", &mut auth))
        .expect("the key refresh succeeds");
    assert_eq!(
        auth.extra.request_route.as_deref(),
        Some("/v1/chat/completions")
    );
    assert!(
        auth.extra.budget_reservation.is_some(),
        "the copy clears the request fields, not the caller's object"
    );
}

#[test]
fn usage_none_noop() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", Some("alias-1"));
    let (outcome, warnings) =
        support::capture(|| runtime.block_on(invalidation.cache_team_object("team-1", &mut team)));
    outcome.unwrap();
    assert!(warnings.is_empty());
    assert_eq!(
        recorder.calls().len(),
        2,
        "the team write and the alias delete still run without a usage cache"
    );
}

#[test]
fn shared_redis_identity_memory_only() {
    let runtime = support::runtime();
    let shared = support::marker();
    let recorder = Recorder::new(Some(Arc::clone(&shared)));
    let usage = Recorder::new(Some(shared));
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert_eq!(
        usage.calls(),
        vec![
            Call::UsageDeleteMemory("team_id:team-1".to_owned()),
            Call::UsageDeleteMemory("team_alias:alias-1".to_owned()),
        ],
        "a shared Redis is cleared by the auth write, so only usage memory is dropped"
    );
}

#[test]
fn both_redis_none_counts_shared() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert!(
        usage
            .calls()
            .iter()
            .all(|call| matches!(call, Call::UsageDeleteMemory(_))),
        "two caches with no Redis share the absence of one"
    );
    assert_eq!(usage.calls().len(), 2);
}

#[test]
fn different_handles_delete_usage_redis() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let usage = Recorder::new(Some(support::marker()));
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert_eq!(
        usage.calls(),
        vec![
            Call::UsageDeleteAsync("team_id:team-1".to_owned()),
            Call::UsageDeleteAsync("team_alias:alias-1".to_owned()),
        ],
        "a usage cache on another Redis is deleted through the async path"
    );
}

#[test]
fn usage_failure_warns_then_team_write() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    usage.fail(Fault::UsageDeleteMemory);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", None);
    let (outcome, warnings) =
        support::capture(|| runtime.block_on(invalidation.cache_team_object("team-1", &mut team)));
    outcome.unwrap();
    assert_eq!(
        support::warned_subjects(&warnings),
        vec!["team_id:team-1".to_owned()]
    );
    assert!(
        recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::AuthWrite { .. })),
        "a usage failure permits the team write"
    );
}

#[test]
fn usage_warning_strips_cr_lf() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    usage.fail(Fault::UsageDeleteMemory);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team\r\n-1", None);
    let (outcome, warnings) = support::capture(|| {
        runtime.block_on(invalidation.cache_team_object("team\r\n-1", &mut team))
    });
    outcome.unwrap();
    let subjects = support::warned_subjects(&warnings);
    assert_eq!(subjects, vec!["team_id:team-1".to_owned()]);
}

#[test]
fn main_write_failure_skips_alias_after_timestamp() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::AuthWrite);
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    let outcome = runtime.block_on(invalidation.cache_team_object("team-1", &mut team));
    outcome.unwrap_err();
    assert_eq!(team.last_refreshed_at, Some(FIXED_NOW));
    assert_eq!(
        usage.calls(),
        vec![Call::UsageDeleteMemory("team_id:team-1".to_owned())],
        "the team usage eviction already happened"
    );
    assert!(
        !support::call_keys(&recorder.calls())
            .iter()
            .any(|key| key.starts_with("team_alias:")),
        "the alias is never touched after the main write fails"
    );
}

#[test]
fn alias_delete_never_alias_write() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert!(
        !recorder.calls().iter().any(|call| matches!(
            call,
            Call::AuthWrite { key, .. } if key.starts_with("team_alias:")
        )),
        "an alias is not unique in Postgres, so refresh never writes an alias mapping"
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn falsey_alias_skips_both_caches(#[case] alias: Option<&str>) {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", alias);
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert!(
        !support::call_keys(&usage.calls())
            .iter()
            .any(|key| key.starts_with("team_alias:")),
        "an alias of {alias:?} skips the usage alias eviction"
    );
    assert!(
        !support::call_keys(&recorder.calls())
            .iter()
            .any(|key| key.starts_with("team_alias:")),
        "an alias of {alias:?} skips the auth alias delete"
    );
}

#[test]
fn alias_queued_delete_is_awaited() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    let calls = recorder.calls();
    assert_eq!(
        calls,
        vec![
            Call::AuthWrite {
                key: "team_id:team-1".to_owned(),
                payload: support::encoded_team(&team),
                ttl: Some(MANAGEMENT_TTL),
            },
            Call::AuthPreCallDelete("team_alias:alias-1".to_owned()),
        ],
        "a queued alias delete is awaited, never repeated directly"
    );
}

#[test]
fn alias_direct_delete_only_without_batch() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::PreCallUnqueued);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert_eq!(
        recorder.calls(),
        vec![
            Call::AuthWrite {
                key: "team_id:team-1".to_owned(),
                payload: support::encoded_team(&team),
                ttl: Some(MANAGEMENT_TTL),
            },
            Call::AuthPreCallDelete("team_alias:alias-1".to_owned()),
            Call::AuthDeleteAsync("team_alias:alias-1".to_owned()),
        ],
        "without a request batch the alias is deleted directly"
    );
}

#[test]
fn alias_failure_warns_then_usage() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    recorder.fail(Fault::PreCallFail);
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut team = support::team("team-1", Some("alias-1"));
    let (outcome, warnings) =
        support::capture(|| runtime.block_on(invalidation.cache_team_object("team-1", &mut team)));
    outcome.unwrap();
    assert_eq!(
        support::warned_subjects(&warnings),
        vec!["team_alias:alias-1".to_owned()]
    );
    assert_eq!(
        usage.calls(),
        vec![
            Call::UsageDeleteMemory("team_id:team-1".to_owned()),
            Call::UsageDeleteMemory("team_alias:alias-1".to_owned()),
        ],
        "usage alias invalidation still runs after an auth alias failure"
    );
}

#[test]
fn refresh_does_not_publish() {
    let runtime = support::runtime();
    let recorder = Recorder::new(None);
    let invalidation = refresh(&recorder, None);
    let mut team = support::team("team-1", Some("alias-1"));
    runtime
        .block_on(invalidation.cache_team_object("team-1", &mut team))
        .expect("the team refresh succeeds");
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Publish { .. } | Call::Evict { .. })),
        "a refresh publishes no invalidation"
    );
}

#[test]
fn key_refresh_no_usage_eviction_or_publish() {
    let runtime = support::runtime();
    let recorder = Recorder::new(Some(support::marker()));
    let usage = Recorder::new(None);
    let invalidation = refresh(&recorder, Some(&usage));
    let mut auth = support::key_auth("/v1/chat/completions");
    runtime
        .block_on(invalidation.cache_key_object("hashed-key-1", &mut auth))
        .expect("the key refresh succeeds");
    assert!(
        usage.calls().is_empty(),
        "a key refresh clears no usage entry"
    );
    assert!(
        !recorder
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Publish { .. } | Call::Evict { .. })),
        "a key refresh broadcasts nothing"
    );
}
