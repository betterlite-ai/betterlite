#![cfg(test)]

mod oracle;
mod stores;

use std::sync::Arc;

use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_config::sections::login_throttle::{
    OverrideLimit, ProxyTopology, Settings, SourceOverride, SourceOverrides,
};
use betterlite_context::local_ttl_cache::CacheValue;
use tracing::subscriber::DefaultGuard;

use super::attempt::Attempt;
use super::error::LoginThrottleError;
use super::keys::{LoginKeys, UNKNOWN_SOURCE};
use super::local::{LocalStore as _, ThrottleStores};
use super::redis::{BlockTtls, FailureLimits, LoginRedis, ThrottleRedisError};
use super::source::{EXEMPT, declared_proxy_ranges, source_group, source_limit};
use super::support::{FakeClock, FakeLocalStore, FakeRedis, WarningLog, classify};
use super::throttle::{LoginThrottle, Scope, ThrottleRuntime};
use super::warnings::WarningCache;

const NO_FAULTS: &[(&str, &str)] = &[];

struct Fixture {
    throttle: LoginThrottle,
    clock: FakeClock,
    counters: Arc<FakeLocalStore>,
    blocks: Arc<FakeLocalStore>,
    redis: Option<Arc<FakeRedis>>,
    warnings: WarningLog,
    _guard: DefaultGuard,
}

fn fixture(settings: &Settings, peer: Option<&str>, now: f64) -> Fixture {
    let request = NetworkRequest {
        peer,
        ..NetworkRequest::default()
    };
    fixture_request(settings, &request, now, None)
}

fn fixture_request(
    settings: &Settings,
    request: &NetworkRequest<'_>,
    now: f64,
    faults: Option<&[(&str, &str)]>,
) -> Fixture {
    let warnings = WarningLog::default();
    let guard = warnings.subscribe();
    let clock = FakeClock::at(now);
    let counters = Arc::new(FakeLocalStore::new(clock.clock()));
    let blocks = Arc::new(FakeLocalStore::new(clock.clock()));
    let stores = ThrottleStores {
        counters: Arc::clone(&counters),
        blocks: Arc::clone(&blocks),
    };
    let redis = faults.map(|entries| Arc::new(FakeRedis::new(clock.clock(), entries)));
    let shared: Option<Arc<dyn LoginRedis>> = redis.as_ref().map(|value| {
        let store: Arc<dyn LoginRedis> = Arc::clone(value);
        store
    });
    let runtime = ThrottleRuntime {
        stores,
        clock: clock.clock(),
        redis: shared,
    };
    let throttle = LoginThrottle::from_request(request, settings, &runtime);
    Fixture {
        throttle,
        clock,
        counters,
        blocks,
        redis,
        warnings,
        _guard: guard,
    }
}

fn direct_topology() -> ProxyTopology {
    ProxyTopology::Entries(Vec::new())
}

fn keys_of(fixture: &Fixture, username: &str) -> LoginKeys {
    LoginKeys::of(&fixture.throttle.client_ip, username)
}

async fn fail(fixture: &Fixture, username: &str) -> BlockTtls {
    fixture
        .throttle
        .record_failure(username)
        .await
        .expect("bump")
}

#[test]
fn mapped_ipv6_is_ipv4() {
    assert_eq!(source_group("::ffff:10.1.2.3"), "10.1.2.3");
    assert_eq!(source_group("10.1.2.3"), "10.1.2.3");
    assert_eq!(
        LoginKeys::of("::ffff:10.1.2.3", "bob"),
        LoginKeys::of("10.1.2.3", "bob")
    );
}

#[test]
fn host_bits_normalized() {
    assert_eq!(source_group("2001:db8:1:2:3:4:5:6"), "2001:db8:1:2::/64");
    assert_eq!(
        source_group("2001:0db8:0001:0002:ffff::1"),
        "2001:db8:1:2::/64"
    );
    assert_eq!(source_group("not-an-address"), "not-an-address");
}

#[test]
fn override_bad_mapping() {
    let warnings = WarningLog::default();
    let _guard = warnings.subscribe();
    let settings = Settings {
        source_limit_overrides: SourceOverrides::Invalid,
        ..Settings::default()
    };
    assert_eq!(source_limit(&settings, "10.1.1.1"), 10);
    // the shape warning belongs to the config section, which reads the raw value once
    assert!(warnings.drain_classified().is_empty());
}

#[test]
fn override_unmatched_value_ignored() {
    let warnings = WarningLog::default();
    let _guard = warnings.subscribe();
    let settings = Settings {
        source_limit_overrides: SourceOverrides::Mapping(vec![SourceOverride {
            range: "192.168.0.0/16".to_owned(),
            limit: OverrideLimit::Invalid,
        }]),
        ..Settings::default()
    };
    assert_eq!(source_limit(&settings, "10.1.1.1"), 10);
    assert!(warnings.drain_classified().is_empty());
    assert_eq!(source_limit(&settings, "192.168.5.5"), 10);
    assert_eq!(
        warnings.drain_classified(),
        vec!["invalid_override_value".to_owned()]
    );
}

#[test]
fn equivalent_range_ties() {
    let settings = Settings {
        source_limit_overrides: SourceOverrides::Mapping(vec![
            SourceOverride {
                range: "10.0.0.5".to_owned(),
                limit: OverrideLimit::Limit(5),
            },
            SourceOverride {
                range: "10.0.0.5/32".to_owned(),
                limit: OverrideLimit::Limit(7),
            },
        ]),
        ..Settings::default()
    };
    assert_eq!(source_limit(&settings, "10.0.0.5"), 7);
    let exempt = Settings {
        source_limit_overrides: SourceOverrides::Mapping(vec![
            SourceOverride {
                range: "10.0.0.5".to_owned(),
                limit: OverrideLimit::Limit(7),
            },
            SourceOverride {
                range: "10.0.0.5/32".to_owned(),
                limit: OverrideLimit::Exempt,
            },
        ]),
        ..Settings::default()
    };
    assert_eq!(source_limit(&exempt, "10.0.0.5"), EXEMPT);
}

#[test]
fn odd_limit_floor() {
    let one = Settings {
        source_limit: 1,
        ..Settings::default()
    };
    assert_eq!(
        fixture(&one, Some("10.1.1.1"), 1000.0).throttle.user_limit,
        1
    );
    let five = Settings {
        source_limit: 5,
        ..Settings::default()
    };
    assert_eq!(
        fixture(&five, Some("10.1.1.1"), 1000.0).throttle.user_limit,
        2
    );
    let seven = Settings {
        source_limit: 7,
        ..Settings::default()
    };
    assert_eq!(
        fixture(&seven, Some("10.1.1.1"), 1000.0)
            .throttle
            .user_limit,
        3
    );
    let ten = Settings {
        source_limit: 10,
        ..Settings::default()
    };
    assert_eq!(
        fixture(&ten, Some("10.1.1.1"), 1000.0).throttle.user_limit,
        5
    );
}

#[test]
fn unicode_pair_hash() {
    let folded = LoginKeys::of("10.1.1.1", "Straße");
    assert_eq!(folded, LoginKeys::of("10.1.1.1", "STRASSE"));
    assert_eq!(folded, LoginKeys::of("10.1.1.1", "strasse"));
    assert_ne!(folded, LoginKeys::of("10.1.1.1", "Strasse!"));
}

#[test]
fn username_spaces_distinct() {
    let base = LoginKeys::of("10.1.1.1", "a b");
    assert_ne!(base, LoginKeys::of("10.1.1.1", "a  b"));
    assert_ne!(base, LoginKeys::of("10.1.1.1", "a b "));
    assert_ne!(
        LoginKeys::of("10.1.1.1", " a"),
        LoginKeys::of("10.1.1.1", "a")
    );
}

#[test]
fn cluster_slot_keys() {
    let keys = LoginKeys::of("10.1.2.3", "bob");
    assert!(keys.pair_counter.starts_with("login_fail:{10.1.2.3}:user:"));
    assert!(
        keys.pair_block
            .starts_with("login_fail:{10.1.2.3}:block:user:")
    );
    assert!(keys.source_counter.starts_with("login_fail:{10.1.2.3}:"));
    assert!(keys.source_block.starts_with("login_fail:{10.1.2.3}:"));
    assert_eq!(keys.source_counter, "login_fail:{10.1.2.3}:source");
    assert_eq!(keys.source_block, "login_fail:{10.1.2.3}:block:source");
}

#[tokio::test]
async fn disabled_attempt_no_effects() {
    let settings = Settings {
        disabled: true,
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert!(!built.throttle.enabled);
    let attempt = match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Allowed(attempt) => attempt,
        Attempt::Refused(_) => panic!("a disabled throttle refuses nothing"),
    };
    attempt.failed().await.expect("disabled failure");
    assert!(
        !built
            .counters
            .contains(&keys_of(&built, "bob").pair_counter)
    );
    assert!(!built.blocks.contains(&keys_of(&built, "bob").pair_block));
}

#[tokio::test]
async fn disabled_setting_cached() {
    let settings = Settings {
        disabled: true,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert!(!built.throttle.enabled);
    // record_failure is not gated on enabled: only LoginAttempt checks the setting
    let ttls = fail(&built, "bob").await;
    assert_eq!(ttls.user, 0);
    assert!(
        built
            .counters
            .contains(&keys_of(&built, "bob").pair_counter)
    );
}

#[tokio::test]
async fn local_shared_maximum() {
    let settings = Settings {
        source_limit: 2,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(NO_FAULTS),
    );
    let keys = keys_of(&built, "bob");
    built
        .blocks
        .set(&keys.pair_block, CacheValue::Float(1300.0), 300)
        .expect("the local block");
    let shared = Arc::clone(built.redis.as_ref().expect("shared"));
    shared
        .record_failure(
            &keys,
            FailureLimits {
                user_limit: 0,
                source_limit: 0,
                window_seconds: 60,
                block_seconds: 900,
            },
        )
        .await
        .expect("shared block");
    match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Refused(refused) => {
            assert_eq!(refused.retry_after, 900);
            assert_eq!(refused.scope, Scope::User);
        }
        Attempt::Allowed(_) => panic!("shared block should refuse"),
    }
    assert_eq!(shared.counter(&keys.pair_counter), Some(1));
    assert_eq!(shared.counter(&keys_of(&built, "dave").pair_counter), None);
    built.clock.set(1061.0);
    assert_eq!(shared.counter(&keys.pair_counter), None);
    let live = shared.block_ttls(&keys).await.expect("the live ttls");
    assert_eq!(live.user, 839);
    built.clock.set(1900.0);
    let expired = shared.block_ttls(&keys).await.expect("the expired ttls");
    assert_eq!(expired.user, -2);
}

#[tokio::test]
async fn source_precedes_longer_pair() {
    let settings = Settings {
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(NO_FAULTS),
    );
    let keys = keys_of(&built, "bob");
    let shared = Arc::clone(built.redis.as_ref().expect("shared store"));
    shared.block(&keys.pair_block, 30);
    shared.block(&keys.source_block, 10);
    match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Refused(refused) => {
            assert_eq!(refused.retry_after, 10);
            assert_eq!(refused.scope, Scope::Source);
        }
        Attempt::Allowed(_) => panic!("source block should refuse"),
    }
}

#[tokio::test]
async fn ttl_rounding_and_expiry() {
    let settings = Settings::default();
    let built = fixture(&settings, Some("10.1.1.1"), 100.5);
    let keys = keys_of(&built, "bob");
    built
        .blocks
        .set(&keys.pair_block, CacheValue::Float(400.5), 300)
        .expect("block");
    built.clock.set(400.4);
    match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Refused(refused) => assert_eq!(refused.retry_after, 1),
        Attempt::Allowed(_) => panic!("a whole second left should refuse"),
    }
    built.clock.set(400.5);
    assert!(matches!(
        built.throttle.attempt("bob").await.expect("attempt"),
        Attempt::Allowed(_)
    ));
}

#[tokio::test]
async fn blocked_attempt_error_shape() {
    let built = fixture(&Settings::default(), Some("10.1.1.1"), 1000.0);
    let keys = keys_of(&built, "bob");
    built
        .blocks
        .set(&keys.pair_block, CacheValue::Float(1010.0), 10)
        .expect("block");
    let refused = match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Refused(refused) => refused,
        Attempt::Allowed(_) => panic!("a blocked pair refuses"),
    };
    let exception = refused.exception();
    assert_eq!(exception.code, "429");
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("username"));
    assert_eq!(
        exception.message,
        "Too many failed sign-in attempts. Try again later."
    );
    assert_eq!(exception.headers.get("Retry-After"), Some(&"10".to_owned()));
    assert_eq!(
        built.warnings.drain_classified(),
        vec!["refused".to_owned()]
    );
}

#[tokio::test]
async fn limit_plus_one_blocks() {
    let settings = Settings {
        source_limit: 6,
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert_eq!(built.throttle.user_limit, 3);
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(fail(&built, "bob").await.user, 300);
    assert_eq!(built.warnings.drain_classified(), Vec::<String>::new());
}

#[tokio::test]
async fn triggering_pair_failure_not_charged_to_source() {
    let settings = Settings {
        source_limit: 4,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    let keys = keys_of(&built, "bob");
    assert_eq!(fail(&built, "bob").await, BlockTtls { user: 0, source: 0 });
    assert_eq!(fail(&built, "bob").await, BlockTtls { user: 0, source: 0 });
    assert_eq!(
        fail(&built, "bob").await,
        BlockTtls {
            user: 300,
            source: 0
        }
    );
    assert_eq!(
        fail(&built, "bob").await,
        BlockTtls {
            user: 300,
            source: 0
        }
    );
    assert_eq!(built.counters.value(&keys.source_counter), Some(2.0_f64));
}

#[tokio::test]
async fn redis_fixed_vs_local_sliding_window() {
    let settings = Settings {
        source_limit: 8,
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(NO_FAULTS),
    );
    let keys = keys_of(&built, "bob");
    assert_eq!(fail(&built, "bob").await.user, 0);
    built.clock.set(1061.0);
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(
        built
            .redis
            .as_ref()
            .expect("shared")
            .counter(&keys.pair_counter),
        Some(1)
    );
}

#[tokio::test]
async fn blocked_failure_does_not_extend() {
    let settings = Settings {
        source_limit: 2,
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(fail(&built, "bob").await.user, 300);
    built.clock.set(1005.0);
    assert_eq!(fail(&built, "bob").await.user, 295);
}

#[tokio::test]
async fn success_preserves_blocks_and_source() {
    let settings = Settings {
        source_limit: 4,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    let keys = keys_of(&built, "bob");
    fail(&built, "bob").await;
    let attempt = match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Allowed(attempt) => attempt,
        Attempt::Refused(_) => panic!("not blocked yet"),
    };
    attempt.succeeded().await.expect("success");
    assert!(!built.counters.contains(&keys.pair_counter));
    assert_eq!(built.counters.value(&keys.source_counter), Some(1.0_f64));
}

#[tokio::test]
async fn redis_failure_classes_fall_back() {
    let settings = Settings {
        source_limit: 4,
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(&[("record_failure", "unavailable")]),
    );
    let keys = keys_of(&built, "bob");
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert_eq!(built.counters.value(&keys.pair_counter), Some(1.0_f64));
    assert_eq!(
        built
            .redis
            .as_ref()
            .expect("shared")
            .counter(&keys.pair_counter),
        None
    );
    assert_eq!(
        built.warnings.drain_classified(),
        vec!["redis_fallback".to_owned()]
    );
}

#[tokio::test]
async fn redis_success_does_not_mirror_local() {
    let settings = Settings {
        source_limit: 4,
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(NO_FAULTS),
    );
    let keys = keys_of(&built, "bob");
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert!(!built.counters.contains(&keys.pair_counter));
    let shared = built.redis.as_ref().expect("shared");
    assert_eq!(shared.counter(&keys.pair_counter), Some(1));
    assert_eq!(shared.counter(&keys.source_counter), None);
    assert!(matches!(
        built.throttle.attempt("bob").await.expect("attempt"),
        Attempt::Allowed(_)
    ));
}

#[tokio::test]
async fn redis_clear_failure_still_clears_local() {
    let settings = Settings::default();
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(&[("delete", "unavailable")]),
    );
    let keys = keys_of(&built, "bob");
    built
        .counters
        .set(&keys.pair_counter, CacheValue::Float(1.0), 60)
        .expect("count");
    built.throttle.clear_pair("bob").await.expect("clear");
    assert!(!built.counters.contains(&keys.pair_counter));
    assert_eq!(
        built.warnings.drain_classified(),
        vec!["redis_fallback".to_owned()]
    );
}

#[tokio::test]
async fn malformed_store_reply_propagates() {
    let settings = Settings::default();
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(&[("block_ttls", "malformed")]),
    );
    let error = match built.throttle.attempt("bob").await {
        Err(error) => error,
        Ok(_) => panic!("a malformed reply must propagate"),
    };
    assert!(matches!(error, LoginThrottleError::Shared(_)));
    let built_record = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(&[("record_failure", "malformed")]),
    );
    let error = match built_record.throttle.record_failure("bob").await {
        Err(error) => error,
        Ok(_) => panic!("a malformed reply must propagate"),
    };
    assert!(matches!(
        error,
        LoginThrottleError::Shared(ThrottleRedisError::MalformedBlockTtls)
    ));
}

#[test]
fn warning_cache_keys() {
    let warnings = WarningLog::default();
    let _guard = warnings.subscribe();
    let cache = WarningCache::default();
    cache.warn_counters_per_worker("4");
    cache.warn_counters_per_worker("4");
    cache.warn_counters_per_worker("8");
    cache.warn_source_login_limit_is_off();
    cache.warn_source_login_limit_is_off();
    assert_eq!(
        warnings.drain_classified(),
        vec![
            "counters_per_worker".to_owned(),
            "counters_per_worker".to_owned(),
            "source_limit_off".to_owned(),
        ]
    );
}

#[tokio::test]
async fn trusted_forwarded_source() {
    let settings = Settings {
        proxy_topology: ProxyTopology::Entries(vec!["10.0.0.0/8".to_owned()]),
        ..Settings::default()
    };
    let forwarded = "203.0.113.9".to_owned();
    let request = NetworkRequest {
        peer: Some("10.1.1.1"),
        x_forwarded_for: Some(&forwarded),
        ..NetworkRequest::default()
    };
    let built = fixture_request(&settings, &request, 1000.0, None);
    assert_eq!(built.throttle.client_ip, "203.0.113.9");
    assert_eq!(built.throttle.source_limit, Some(10));
}

#[tokio::test]
async fn missing_source_pair_only() {
    let settings = Settings::default();
    let built = fixture(&settings, None, 1000.0);
    assert_eq!(built.throttle.client_ip, UNKNOWN_SOURCE);
    assert_eq!(built.throttle.source_limit, None);
    assert_eq!(built.throttle.user_limit, 5);
    let keys = LoginKeys::of(UNKNOWN_SOURCE, "bob");
    assert_eq!(fail(&built, "bob").await.user, 0);
    assert!(built.counters.contains(&keys.pair_counter));
}

#[test]
fn topology_blank_or_malformed_entry() {
    let blank = ProxyTopology::Entries(vec!["10.0.0.0/8".to_owned(), String::new()]);
    assert_eq!(declared_proxy_ranges(&blank), None);
    let malformed = ProxyTopology::Entries(vec!["not-a-net".to_owned()]);
    assert_eq!(declared_proxy_ranges(&malformed), None);
    assert_eq!(
        declared_proxy_ranges(&ProxyTopology::Entries(Vec::new())),
        Some(Vec::new())
    );
    assert_eq!(declared_proxy_ranges(&ProxyTopology::Unknown), None);
}

#[test]
fn scoped_addresses() {
    assert_eq!(source_group("fe80::1%eth0"), "fe80::/64");
    assert_eq!(source_group("10.1.2.3%eth0"), "10.1.2.3%eth0");
    assert_eq!(source_group("fe80::1%"), "fe80::1%");
    assert_eq!(source_group("::%a%b"), "::%a%b");
    assert_eq!(source_group("::%"), "::%");
}

#[test]
fn local_store_expires_on_read() {
    let clock = FakeClock::at(1000.0);
    let store = FakeLocalStore::new(clock.clock());
    store.set("k", CacheValue::Float(1.0), 10).expect("the set");
    clock.set(1005.0);
    assert_eq!(
        store.get("k").expect("the live read"),
        Some(CacheValue::Float(1.0))
    );
    clock.set(1010.0);
    assert!(store.get("k").expect("the expired read").is_none());
}

#[test]
fn local_store_set_keeps_a_live_ttl() {
    let clock = FakeClock::at(1000.0);
    let store = FakeLocalStore::new(clock.clock());
    store
        .set("k", CacheValue::Float(1.0), 10)
        .expect("the first set");
    clock.set(1005.0);
    store
        .set("k", CacheValue::Float(2.0), 60)
        .expect("the live reset");
    clock.set(1011.0);
    assert!(store.get("k").expect("the kept expiry").is_none());

    let exact = FakeClock::at(1000.0);
    let other = FakeLocalStore::new(exact.clock());
    other
        .set("k", CacheValue::Float(1.0), 10)
        .expect("the first set");
    exact.set(1010.0);
    other
        .set("k", CacheValue::Float(2.0), 60)
        .expect("the reset at the expiry");
    exact.set(1020.0);
    assert!(matches!(
        other.get("k").expect("the refreshed read"),
        Some(CacheValue::Float(_))
    ));
}

#[tokio::test]
async fn exempt_override_disables_the_throttle() {
    let settings = Settings {
        source_limit_overrides: SourceOverrides::Mapping(vec![SourceOverride {
            range: "10.1.1.1".to_owned(),
            limit: OverrideLimit::Exempt,
        }]),
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert!(!built.throttle.enabled);
    assert_eq!(built.throttle.source_limit, None);
    assert_eq!(built.throttle.user_limit, 1);
    let attempt = match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Allowed(attempt) => attempt,
        Attempt::Refused(_) => panic!("an exempt source is never refused"),
    };
    attempt.failed().await.expect("the exempt failure");
    assert!(
        !built
            .counters
            .contains(&keys_of(&built, "bob").pair_counter)
    );
}

#[tokio::test]
async fn undeclared_topology_ignores_a_source_block() {
    let settings = Settings {
        source_limit: 4,
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert_eq!(built.throttle.source_limit, None);
    assert_eq!(built.throttle.user_limit, 2);
    built
        .blocks
        .set(
            &keys_of(&built, "bob").source_block,
            CacheValue::Float(1300.0),
            300,
        )
        .expect("the source block");
    assert!(matches!(
        built.throttle.attempt("bob").await.expect("attempt"),
        Attempt::Allowed(_)
    ));
}

#[tokio::test]
async fn shared_source_counts_when_declared() {
    let settings = Settings {
        source_limit: 6,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture_request(
        &settings,
        &NetworkRequest {
            peer: Some("10.1.1.1"),
            ..NetworkRequest::default()
        },
        1000.0,
        Some(NO_FAULTS),
    );
    let keys = keys_of(&built, "bob");
    let shared = Arc::clone(built.redis.as_ref().expect("shared"));
    assert_eq!(fail(&built, "bob").await, BlockTtls { user: 0, source: 0 });
    assert_eq!(fail(&built, "bob").await, BlockTtls { user: 0, source: 0 });
    assert_eq!(fail(&built, "bob").await, BlockTtls { user: 0, source: 0 });
    assert_eq!(
        fail(&built, "bob").await,
        BlockTtls {
            user: 300,
            source: 0
        }
    );
    assert_eq!(shared.counter(&keys.pair_counter), Some(4));
    assert_eq!(shared.counter(&keys.source_counter), Some(3));
    built.clock.set(1005.0);
    assert_eq!(
        fail(&built, "bob").await,
        BlockTtls {
            user: 295,
            source: 0
        }
    );
    assert_eq!(shared.counter(&keys.pair_counter), Some(4));
    assert_eq!(
        fail(&built, "carol").await,
        BlockTtls { user: 0, source: 0 }
    );
    assert_eq!(
        fail(&built, "carol").await,
        BlockTtls { user: 0, source: 0 }
    );
    assert_eq!(
        fail(&built, "carol").await,
        BlockTtls { user: 0, source: 0 }
    );
    assert_eq!(
        fail(&built, "carol").await,
        BlockTtls {
            user: 300,
            source: 0
        }
    );
    assert_eq!(
        fail(&built, "dave").await,
        BlockTtls {
            user: 0,
            source: 300
        }
    );
    assert_eq!(shared.counter(&keys.source_counter), Some(7));
    match built.throttle.attempt("dave").await.expect("attempt") {
        Attempt::Refused(refused) => {
            assert_eq!(refused.scope, Scope::Source);
            assert_eq!(refused.retry_after, 300);
        }
        Attempt::Allowed(_) => panic!("the source block should refuse"),
    }
    built.throttle.clear_pair("bob").await.expect("the clear");
    assert_eq!(shared.counter(&keys.pair_counter), None);
    assert_eq!(shared.counter(&keys.source_counter), Some(7));
}

#[tokio::test]
async fn failed_attempt_records_and_warns() {
    let settings = Settings {
        source_limit: 4,
        proxy_topology: direct_topology(),
        ..Settings::default()
    };
    let built = fixture(&settings, Some("10.1.1.1"), 1000.0);
    assert_eq!(built.throttle.user_limit, 2);
    fail_attempt(&built, "bob").await;
    fail_attempt(&built, "bob").await;
    assert!(built.warnings.drain_classified().is_empty());
    fail_attempt(&built, "bob").await;
    let bob = built.warnings.messages();
    assert_eq!(bob.len(), 1);
    assert!(bob[0].contains("for 300 seconds"), "{bob:?}");
    assert!(bob[0].contains("scope=user username=\"bob\""), "{bob:?}");
    built.warnings.drain_classified();
    fail_attempt(&built, "carol").await;
    fail_attempt(&built, "carol").await;
    fail_attempt(&built, "carol").await;
    built.warnings.drain_classified();
    fail_attempt(&built, "dave").await;
    let dave = built.warnings.messages();
    assert_eq!(dave.len(), 1);
    assert!(dave[0].contains("for 300 seconds"), "{dave:?}");
    assert!(
        dave[0].contains("scope=source username=\"dave\""),
        "{dave:?}"
    );
    built.warnings.drain_classified();
    match built.throttle.attempt("bob").await.expect("attempt") {
        Attempt::Refused(refused) => {
            assert_eq!(refused.scope, Scope::User);
            assert_eq!(refused.retry_after, 300);
        }
        Attempt::Allowed(_) => panic!("bob's pair block should refuse"),
    }
    match built.throttle.attempt("dave").await.expect("attempt") {
        Attempt::Refused(refused) => assert_eq!(refused.scope, Scope::Source),
        Attempt::Allowed(_) => panic!("dave's source should refuse"),
    }
}

async fn fail_attempt(fixture: &Fixture, username: &str) {
    let attempt = match fixture.throttle.attempt(username).await.expect("attempt") {
        Attempt::Allowed(attempt) => attempt,
        Attempt::Refused(_) => panic!("{username} is not blocked yet"),
    };
    attempt.failed().await.expect("the failed attempt");
}

#[test]
fn classification_tokens_are_stable() {
    assert_eq!(classify("Redis failed while counting"), "redis_fallback");
    assert_eq!(classify("Admin UI sign-in refused: the user"), "refused");
    assert_eq!(
        classify("Invalid failed_login_window_seconds value 0"),
        "invalid_window"
    );
    assert_eq!(classify("something else"), "other:something else");
}
