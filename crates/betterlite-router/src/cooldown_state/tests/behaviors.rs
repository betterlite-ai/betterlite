use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::support::{
    MemoryCooldowns, MemoryHealth, TestClock, Written, active_at, cache, deployment, flagged,
    health, ids, kept, key, pool, read_at, record, settings, snapshot, unhealthy, unhealthy_ids,
};
use crate::cooldown_state::cooldown::ActiveCooldown;
use crate::cooldown_state::filter::{
    HealthRouting, all_blocked, fully_blocked_model_names, fully_unhealthy_model_names,
    without_cooling_down, without_unhealthy,
};
use crate::cooldown_state::health::HealthSnapshot;
use crate::cooldown_state::record::CooldownRecord;
use crate::routing_read_batch::store::ReadWarning;

pub(crate) async fn run(name: &str) {
    match name {
        "add_masks_the_exception_and_writes_both_tiers" => add_masks_the_exception(),
        "add_uses_an_explicit_cooldown_time" => add_uses_an_explicit_cooldown_time(),
        "read_answers_from_memory_without_a_shared_read" => {
            read_answers_from_memory().await;
        }
        "read_reads_the_shared_tier_once_for_every_due_key" => read_reads_the_shared_tier().await,
        "a_warning_from_the_shared_tier_is_not_an_error" => a_warning_is_not_an_error().await,
        "an_expired_cooldown_leaves_the_memory_tier" => an_expired_cooldown_leaves().await,
        "a_promoted_entry_keeps_the_remaining_time" => a_promoted_entry_is_corrected().await,
        "an_unexpired_local_ttl_survives_a_second_add" => a_second_add_keeps_the_local_ttl(),
        "a_re_add_after_the_local_ttl_expired_takes_the_new_ttl" => a_second_add_after_expiry(),
        "local_expiries_report_the_memory_tier_deadlines" => local_expiries_report(),
        "set_states_keeps_an_unexpired_local_ttl" => set_states_keeps_the_local_ttl(),
        "the_minimum_cooldown_is_the_smallest_configured_time" => minimum_cooldown_is_smallest(),
        "the_minimum_cooldown_falls_back_to_the_router_default" => minimum_cooldown_falls_back(),
        "set_states_merges_and_prunes_the_expired" => set_states_merges(),
        "set_states_prefers_the_shared_snapshot" => set_states_prefers_shared(),
        "set_states_falls_back_to_the_local_copy" => set_states_falls_back(),
        "a_missing_health_snapshot_marks_nothing_unhealthy" => {
            missing_snapshot_marks_nothing().await
        }
        "an_explicitly_healthy_state_is_not_unhealthy" => a_healthy_state_is_not_unhealthy().await,
        "the_sync_facade_answers_from_the_shared_snapshot" => the_sync_facade_answers(),
        other => run_pool(other),
    }
}

type SyncCase = (&'static str, fn());

const SYNC_CASES: &[SyncCase] = &[
    (
        "cooling_deployments_leave_the_pool",
        cooling_deployments_leave_the_pool,
    ),
    (
        "the_health_filter_is_a_no_op_until_it_is_enabled",
        the_filter_is_opt_in,
    ),
    (
        "an_allowed_fails_policy_skips_the_health_filter",
        a_policy_skips_the_filter,
    ),
    (
        "an_unhealthy_deployment_leaves_the_pool",
        an_unhealthy_deployment_leaves,
    ),
    (
        "unlisted_groups_keep_their_deployments_when_scoped",
        unlisted_groups_keep_theirs,
    ),
    (
        "the_health_probe_gets_an_empty_pool_when_every_candidate_is_unhealthy",
        the_probe_gets_an_empty_pool,
    ),
    (
        "a_request_keeps_its_candidates_when_every_one_is_unhealthy",
        a_request_keeps_every_candidate,
    ),
    (
        "a_model_with_every_deployment_paused_is_listed",
        a_paused_model_is_listed,
    ),
    (
        "a_fully_unhealthy_model_is_listed_with_its_team_alias",
        a_fully_unhealthy_model_is_listed,
    ),
    (
        "an_allowed_fails_policy_hides_no_model",
        a_policy_hides_no_model,
    ),
];

fn run_pool(name: &str) {
    match SYNC_CASES
        .iter()
        .find(|(case_name, _run)| *case_name == name)
        .copied()
    {
        Some((_case, run)) => run(),
        None => panic!("unknown case {name}"),
    }
}

fn add_masks_the_exception() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    clock.set(100.0);
    cache.add("a", &"x".repeat(60), 429, None).unwrap();
    assert_eq!(
        shared.writes(),
        vec![Written {
            key: key("a"),
            record: CooldownRecord {
                exception_received: format!("{}{}", "x".repeat(50), "*".repeat(10)),
                status_code: String::from("429"),
                timestamp: 100.0,
                cooldown_time: 5.0,
            },
            ttl_seconds: 5.0,
        }]
    );
}

fn add_uses_an_explicit_cooldown_time() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache
        .add("a", "rate limited", 500, Some(2.0))
        .expect("the cooldown is stored");
    assert_eq!(
        shared.writes(),
        vec![Written {
            key: key("a"),
            record: CooldownRecord {
                exception_received: String::from("rate limited"),
                status_code: String::from("500"),
                timestamp: 0.0,
                cooldown_time: 2.0,
            },
            ttl_seconds: 2.0,
        }]
    );
}

async fn read_answers_from_memory() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache.add("a", "boom", 429, None).unwrap();
    let results = read_at(&cache, &clock, 1.0, &["a"]).await;
    assert_eq!(results, vec![Some(record(0.0, 5.0))]);
    assert!(shared.reads().is_empty());
}

async fn read_reads_the_shared_tier() {
    let clock = TestClock::new();
    let stored = MemoryCooldowns::with(&[("a", record(0.0, 5.0)), ("b", record(0.0, 9.0))]);
    let shared = Arc::new(stored);
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    let results = read_at(&cache, &clock, 1.0, &["a", "b"]).await;
    assert_eq!(
        results,
        vec![Some(record(0.0, 5.0)), Some(record(0.0, 9.0))]
    );
    assert_eq!(shared.reads(), vec![vec![key("a"), key("b")]]);
}

async fn a_warning_is_not_an_error() {
    let clock = TestClock::new();
    let stored =
        MemoryCooldowns::with(&[("a", record(0.0, 5.0))]).warning(ReadWarning::Unavailable);
    let shared = Arc::new(stored);
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    let results = read_at(&cache, &clock, 1.0, &["a"]).await;
    assert_eq!(results, vec![None]);
    assert_eq!(shared.reads(), vec![vec![key("a")]]);
}

async fn an_expired_cooldown_leaves() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 5.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(
        active_at(&cache, &clock, 1.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 4.0,
            record: record(0.0, 5.0),
        }]
    );
    shared.set("a", record(95.0, 30.0));
    assert_eq!(
        active_at(&cache, &clock, 100.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 25.0,
            record: record(95.0, 30.0),
        }]
    );
}

async fn a_promoted_entry_is_corrected() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::with(&[("a", record(0.0, 90.0))]));
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    assert_eq!(
        active_at(&cache, &clock, 1.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 89.0,
            record: record(0.0, 90.0),
        }]
    );
    shared.set("a", record(65.0, 30.0));
    assert_eq!(
        active_at(&cache, &clock, 70.0, "a").await,
        vec![ActiveCooldown {
            model_id: String::from("a"),
            remaining_seconds: 25.0,
            record: record(65.0, 30.0),
        }]
    );
}

fn a_second_add_keeps_the_local_ttl() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache.add("a", "boom", 429, Some(5.0)).unwrap();
    clock.set(2.0);
    cache.add("a", "boom again", 500, Some(30.0)).unwrap();
    assert_eq!(cache.local_expiries(&ids(&["a"])).unwrap(), vec![Some(5.0)]);
    assert_eq!(shared.writes()[1].ttl_seconds.to_bits(), 30.0_f64.to_bits());
    assert_eq!(
        shared.writes()[1].record,
        CooldownRecord {
            exception_received: String::from("boom again"),
            status_code: String::from("500"),
            timestamp: 2.0,
            cooldown_time: 30.0,
        }
    );
}

fn a_second_add_after_expiry() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache.add("a", "boom", 429, Some(5.0)).unwrap();
    clock.set(6.0);
    cache.add("a", "boom again", 429, Some(30.0)).unwrap();
    assert_eq!(
        cache.local_expiries(&ids(&["a"])).unwrap(),
        vec![Some(36.0)]
    );
}

fn local_expiries_report() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryCooldowns::default());
    let cache = cache(&clock, Arc::clone(&shared), &settings());
    cache.add("a", "boom", 429, Some(5.0)).unwrap();
    cache.add("b", "boom", 429, Some(9.0)).unwrap();
    clock.set(5.0);
    assert_eq!(
        cache.local_expiries(&ids(&["a", "b", "c"])).unwrap(),
        vec![Some(5.0), Some(9.0), None]
    );
    clock.set(9.5);
    assert_eq!(
        cache.local_expiries(&ids(&["a", "b"])).unwrap(),
        vec![None, None]
    );
}

fn set_states_keeps_the_local_ttl() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    cache
        .set_states(&snapshot(&[("a", unhealthy(0.0))]))
        .unwrap();
    assert_eq!(cache.local_expiry(), Some(15.0));
    clock.set(1.0);
    cache
        .set_states(&snapshot(&[("b", unhealthy(1.0))]))
        .unwrap();
    assert_eq!(cache.local_expiry(), Some(15.0));
    clock.set(16.0);
    cache
        .set_states(&snapshot(&[("c", unhealthy(16.0))]))
        .unwrap();
    assert_eq!(cache.local_expiry(), Some(31.0));
}

fn minimum_cooldown_is_smallest() {
    let clock = TestClock::new();
    let cache = cache(&clock, Arc::new(MemoryCooldowns::default()), &settings());
    let results = [
        Some(record(0.0, 5.0)),
        Some(record(0.0, 2.5)),
        Some(record(0.0, 7.0)),
    ];
    assert_eq!(
        cache.minimum_cooldown(&results).to_bits(),
        2.5_f64.to_bits()
    );
}

fn minimum_cooldown_falls_back() {
    let clock = TestClock::new();
    let mut configured = settings();
    configured.default_cooldown_time = 12.0;
    let cache = cache(&clock, Arc::new(MemoryCooldowns::default()), &configured);
    assert_eq!(
        cache.minimum_cooldown(&[None, None]).to_bits(),
        12.0_f64.to_bits()
    );
}

fn set_states_merges() {
    let clock = TestClock::new();
    let existing = snapshot(&[("a", unhealthy(100.0)), ("b", unhealthy(0.0))]);
    let shared = Arc::new(MemoryHealth::with(existing));
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    clock.set(100.0);
    cache
        .set_states(&snapshot(&[("c", unhealthy(100.0))]))
        .unwrap();
    assert_eq!(
        shared.writes(),
        vec![(
            snapshot(&[("a", unhealthy(100.0)), ("c", unhealthy(100.0))]),
            15
        )]
    );
}

fn set_states_prefers_shared() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    cache
        .set_states(&snapshot(&[("a", unhealthy(0.0))]))
        .unwrap();
    shared.set_raw(snapshot(&[("b", unhealthy(1.0))]));
    cache
        .set_states(&snapshot(&[("c", unhealthy(1.0))]))
        .unwrap();
    let writes = shared.writes();
    assert_eq!(writes.len(), 2);
    let merged = &writes[1].0;
    assert!(merged.contains_key("b"));
    assert!(merged.contains_key("c"));
    assert!(!merged.contains_key("a"));
}

fn set_states_falls_back() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    cache
        .set_states(&snapshot(&[("a", unhealthy(0.0))]))
        .unwrap();
    shared.clear_raw();
    clock.set(1.0);
    cache
        .set_states(&snapshot(&[("c", unhealthy(1.0))]))
        .unwrap();
    assert_eq!(
        shared
            .writes()
            .last()
            .map(|(merged, ttl_seconds)| (merged.len(), *ttl_seconds)),
        Some((2, 15))
    );
    assert_eq!(shared.writes()[1].0.get("a"), Some(&unhealthy(0.0)));
}

async fn missing_snapshot_marks_nothing() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::default());
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    clock.set(50.0);
    assert!(cache.unhealthy().await.is_empty());
    assert!(cache.unhealthy_now().is_empty());
}

async fn a_healthy_state_is_not_unhealthy() {
    let clock = TestClock::new();
    let stored = snapshot(&[("a", healthy(1.0)), ("b", unhealthy(1.0))]);
    let shared = Arc::new(MemoryHealth::with(stored));
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    clock.set(2.0);
    assert_eq!(cache.unhealthy().await, unhealthy_ids(&["b"]));
    assert_eq!(cache.unhealthy_now(), unhealthy_ids(&["b"]));
    assert_eq!(shared.read_count(), 1);
    // a state written without a verdict is healthy, which is how DeploymentHealthStateValue defaults it
    let decoded: HealthSnapshot =
        serde_json::from_str(r#"{"a": {"timestamp": 1.0}}"#).expect("the state decodes");
    assert_eq!(decoded.get("a").map(|state| state.is_healthy), Some(true));
}

fn the_sync_facade_answers() {
    let clock = TestClock::new();
    let shared = Arc::new(MemoryHealth::with(snapshot(&[("a", unhealthy(1.0))])));
    let cache = health(&clock, Arc::clone(&shared), 10.0);
    clock.set(2.0);
    assert_eq!(cache.unhealthy_now(), unhealthy_ids(&["a"]));
    assert_eq!(shared.read_count(), 0);
}

fn cooling_deployments_leave_the_pool() {
    let deployments = pool(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let result = without_cooling_down(deployments, &ids(&["a"]));
    assert_eq!(kept(&result), ids(&["b"]));
}

fn the_filter_is_opt_in() {
    let deployments = pool(&[("gpt-4o", "a")]);
    let routing = HealthRouting {
        enabled: false,
        has_allowed_fails_policy: false,
        scoped_groups: None,
    };
    let result = without_unhealthy(deployments, &unhealthy_ids(&["a"]), routing, false);
    assert_eq!(kept(&result), ids(&["a"]));
}

fn a_policy_skips_the_filter() {
    let deployments = pool(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let exempt = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: true,
        scoped_groups: None,
    };
    let exempt_pool = without_unhealthy(deployments.clone(), &unhealthy_ids(&["a"]), exempt, false);
    assert_eq!(kept(&exempt_pool), ids(&["a", "b"]));
    let filtered = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: false,
        scoped_groups: None,
    };
    let filtered_pool = without_unhealthy(deployments, &unhealthy_ids(&["a"]), filtered, false);
    assert_eq!(kept(&filtered_pool), ids(&["b"]));
}

fn an_unhealthy_deployment_leaves() {
    let deployments = pool(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let routing = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: false,
        scoped_groups: None,
    };
    let result = without_unhealthy(deployments, &unhealthy_ids(&["a"]), routing, false);
    assert_eq!(kept(&result), ids(&["b"]));
}

fn unlisted_groups_keep_theirs() {
    let deployments = pool(&[("gpt-4o", "a"), ("claude", "b")]);
    let groups: BTreeSet<String> = std::iter::once(String::from("gpt-4o")).collect();
    let routing = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: true,
        scoped_groups: Some(&groups),
    };
    let result = without_unhealthy(deployments, &unhealthy_ids(&["a", "b"]), routing, false);
    assert_eq!(kept(&result), ids(&["b"]));
}

fn the_probe_gets_an_empty_pool() {
    let deployments = pool(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let routing = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: false,
        scoped_groups: None,
    };
    let result = without_unhealthy(deployments, &unhealthy_ids(&["a", "b"]), routing, true);
    assert!(result.is_empty());
}

fn a_request_keeps_every_candidate() {
    let deployments = pool(&[("gpt-4o", "a"), ("gpt-4o", "b")]);
    let routing = HealthRouting {
        enabled: true,
        has_allowed_fails_policy: false,
        scoped_groups: None,
    };
    let result = without_unhealthy(deployments, &unhealthy_ids(&["a", "b"]), routing, false);
    assert_eq!(kept(&result), ids(&["a", "b"]));
}

fn a_paused_model_is_listed() {
    let paused = |model_name: &str, id: &str| {
        flagged(deployment(model_name, id), "blocked", Json::Bool(true))
    };
    let deployments = vec![
        paused("gpt-4o", "a"),
        paused("gpt-4o", "b"),
        deployment("claude", "c"),
        paused("mixed", "d"),
        deployment("mixed", "e"),
    ];
    assert_eq!(
        fully_blocked_model_names(&deployments),
        unhealthy_ids(&["gpt-4o"])
    );
    assert!(all_blocked(&deployments[..2]));
    assert!(!all_blocked(&deployments));
    assert!(!all_blocked(&[]));
}

fn a_fully_unhealthy_model_is_listed() {
    let aliased = flagged(
        deployment("gpt-4o", "a"),
        "team_public_model_name",
        Json::Text(String::from("team-gpt")),
    );
    let deployments = vec![
        aliased,
        deployment("gpt-4o", "b"),
        deployment("claude", "c"),
        deployment("mixed", "d"),
        deployment("mixed", "e"),
    ];
    let unhealthy = unhealthy_ids(&["a", "b", "d"]);
    assert_eq!(
        fully_unhealthy_model_names(&deployments, &unhealthy, false),
        unhealthy_ids(&["gpt-4o", "team-gpt"])
    );
}

fn a_policy_hides_no_model() {
    let deployments = vec![deployment("gpt-4o", "a")];
    let unhealthy = unhealthy_ids(&["a"]);
    assert!(fully_unhealthy_model_names(&deployments, &unhealthy, true).is_empty());
}
