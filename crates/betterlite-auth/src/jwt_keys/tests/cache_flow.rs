use std::sync::Arc;

use betterlite_models::decisions::Json;
use tracing::Level;

use crate::jwt_keys::errors::{KeyError, exception_for};
use crate::jwt_keys::resolver::JwksResolver;

use super::support::{
    FakeCache, FakeClock, JWKS_URL, KEYS_KEY, LevelCapture, Reply, ScriptedHttp, fixture, json,
    key, keys, outage, resolver_over, settings, stale_key, timestamp_key, unreachable_key,
};

#[tokio::test]
async fn fresh_cache() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    let first = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(first, key("a"));
    let second = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(second, key("a"));
    assert_eq!(fixture.http.requests(), vec![JWKS_URL.to_owned()]);
    let writes = fixture.cache.writes();
    assert_eq!(writes.len(), 3);
    assert_eq!(writes[0].key, KEYS_KEY);
    assert!(writes[0].ttl.total_cmp(&600.0_f64).is_eq());
    assert_eq!(writes[1].key, stale_key());
    assert!(writes[1].ttl.total_cmp(&4_200.0_f64).is_eq());
    assert_eq!(writes[2].key, timestamp_key());
    assert!(writes[2].ttl.total_cmp(&4_200.0_f64).is_eq());
    assert_eq!(
        writes[2].value,
        Json::Number(serde_json::Number::from_f64(1_000.0).unwrap())
    );
}

#[tokio::test]
async fn fresh_nonnull_and_lock_recheck() {
    // a stored null reads back as None in Python, so it is a miss and the resolver refetches
    let null_hit = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    null_hit.cache.seed(KEYS_KEY, Json::Null);
    let value = null_hit.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert_eq!(null_hit.http.requests().len(), 1);

    // a value written while this caller waited for the refresh lock is served by the recheck
    let recheck = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, Vec::new());
    recheck.cache.seed(KEYS_KEY, keys(&[key("a")]));
    recheck.cache.miss_once(KEYS_KEY);
    let value = recheck.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert!(recheck.http.requests().is_empty());
}

#[tokio::test]
async fn numeric_timestamp_not_string() {
    // a timestamp that is not a number is not an age, so the stale copy is not servable
    for written_at in [
        Json::Text("1000".to_owned()),
        Json::Null,
        json(r#"{"at":1000}"#),
        json("[1000]"),
    ] {
        let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
        fixture.cache.seed(&stale_key(), keys(&[key("a")]));
        fixture.cache.seed(&timestamp_key(), written_at);
        let error = fixture
            .resolver
            .get_public_key(Some("a"))
            .await
            .unwrap_err();
        assert!(matches!(error, KeyError::Unreachable { .. }));
    }

    // Python's bool is an int, so true is the epoch 1.0 and is served inside the window
    let boolean = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    boolean.cache.seed(&stale_key(), keys(&[key("a")]));
    boolean.cache.seed(&timestamp_key(), Json::Bool(true));
    let value = boolean.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));

    // and a numeric timestamp is an age: 100 seconds old inside a 4200 second lifetime
    let numeric = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    numeric.cache.seed(&stale_key(), keys(&[key("a")]));
    numeric.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(900.0).unwrap()),
    );
    let value = numeric.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
}

#[tokio::test]
async fn stale_age_equal_negative_missing() {
    let window = |offset: f64| {
        let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
        fixture.cache.seed(&stale_key(), keys(&[key("a")]));
        fixture.cache.seed(
            &timestamp_key(),
            Json::Number(serde_json::Number::from_f64(1_000.0 - offset).unwrap()),
        );
        fixture
    };

    // age == ttl + stale_ttl is still inside the window
    let equal = window(4_200.0);
    let value = equal.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));

    // one hair past it is not
    let over = window(4_200.5);
    let error = over.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));

    // a future write time is a negative age, which is inside the window
    let future = window(-30.0);
    let value = future.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));

    // the copy needs both halves of the record
    let no_timestamp = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    no_timestamp.cache.seed(&stale_key(), keys(&[key("a")]));
    let error = no_timestamp
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));

    let no_copy = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    no_copy.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(1_000.0).unwrap()),
    );
    let error = no_copy
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
}

#[tokio::test]
async fn live_ttl_reduction_applies_to_old_copy() {
    let warm = |stale_ttl: f64, ttl: f64| -> (JwksResolver, Arc<FakeCache>) {
        let cache = Arc::new(FakeCache::default());
        cache.seed(&stale_key(), keys(&[key("a")]));
        cache.seed(
            &timestamp_key(),
            Json::Number(serde_json::Number::from_f64(1_000.0).unwrap()),
        );
        let http = Arc::new(ScriptedHttp::new(outage()));
        let clock = Arc::new(FakeClock::new(1_100.0));
        (
            resolver_over(
                settings(Some(JWKS_URL), ttl, stale_ttl),
                Arc::clone(&cache),
                http,
                clock,
            ),
            cache,
        )
    };

    // the copy is 100 seconds old: still inside the 600 + 3600 window it was written under
    let (long, _) = warm(3_600.0, 600.0);
    let value = long.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));

    // the same copy under a lowered window (50 + 30) is past it, so the operator's cut takes effect now
    let (short, short_cache) = warm(30.0, 50.0);
    let error = short.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
    assert_eq!(short_cache.writes().len(), 1);
}

#[tokio::test]
async fn cache_write_order_and_partial_failure() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(
        fixture.cache.write_keys(),
        vec![KEYS_KEY.to_owned(), stale_key(), timestamp_key()]
    );

    // a failure writing the stale copy leaves the fresh entry in place and surfaces the cache error
    let partial = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    *partial.cache.fail_set.lock().unwrap() = Some(stale_key());
    let error = partial
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Cache(_)));
    assert!(partial.cache.contains(KEYS_KEY));
    assert!(!partial.cache.contains(&stale_key()));
    assert!(!partial.cache.contains(&timestamp_key()));
    assert_eq!(partial.cache.write_keys(), vec![KEYS_KEY.to_owned()]);
    let served = partial.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(served, keys(&[key("a")]));

    // a failure writing the fresh entry happens before the stale copy is even attempted
    let refused = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    *refused.cache.fail_set.lock().unwrap() = Some(KEYS_KEY.to_owned());
    let error = refused
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Cache(_)));
    assert!(refused.cache.writes().is_empty());
}

#[tokio::test]
async fn only_transport_outage_is_backed_off() {
    // transport failures are the one outage class, and they store the message for 30 seconds
    let transport = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    let error = transport
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    let KeyError::Unreachable { message } = &error else {
        panic!("a transport outage is unreachable, got {error:?}");
    };
    assert!(message.contains("after 3 attempts"));
    let writes = transport.cache.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key, unreachable_key());
    assert_eq!(writes[0].value, Json::Text(message.clone()));
    assert!(writes[0].ttl.total_cmp(&30.0_f64).is_eq());

    // the stored marker answers the next call without a fetch, with the stored message
    let again = transport
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(again, KeyError::Unreachable { message: stored } if stored == *message));
    assert_eq!(transport.http.requests().len(), 3);

    // a non-transport failure is an answer: one attempt, no marker
    let failed = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![Reply::Failed],
    );
    let error = failed.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::Fetch { .. }));
    assert_eq!(failed.http.requests().len(), 1);
    assert!(failed.cache.writes().is_empty());

    // a status is an answer too
    let status = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(500, "boom")],
    );
    let error = status.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::JwksStatus { status: 500, .. }));
    assert!(status.cache.writes().is_empty());

    // a stored marker that is not a message still stops the fetch, with the fallback wording
    let odd = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, Vec::new());
    odd.cache
        .seed(&unreachable_key(), json(r#"{"failed":true}"#));
    let error = odd.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(
        error,
        KeyError::Unreachable { message } if message == "a previous JWKS fetch failed"
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn stale_warning_does_not_extend_life() {
    let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    fixture.cache.seed(&stale_key(), keys(&[key("a")]));
    fixture.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(1_000.0 - 4_100.0).unwrap()),
    );
    let capture = LevelCapture::new();
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
    let levels = capture.levels();

    // two retry warnings and the stale warning, and no write that would renew the copy
    assert_eq!(
        levels.iter().filter(|level| **level == Level::WARN).count(),
        3
    );
    assert_eq!(
        fixture.cache.write_keys(),
        vec![unreachable_key()],
        "serving stale must not rewrite the copy or its timestamp"
    );

    // the age is measured from the original write, not from the stale serve, so just past the window it fails
    drop(capture);
    fixture.clock.advance(101.0);
    fixture.cache.remove(&unreachable_key());
    fixture.http.push_all(outage());
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
}

#[tokio::test]
async fn stale_on_transient_outage() {
    let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    fixture.cache.seed(&stale_key(), keys(&[key("a")]));
    fixture.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(300.0).unwrap()),
    );
    let capture = LevelCapture::new();
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
    assert_eq!(fixture.http.requests().len(), 3);
    assert!(capture.levels().contains(&Level::WARN));
    assert_eq!(fixture.cache.write_keys(), vec![unreachable_key()]);
}

#[tokio::test]
async fn stale_window_exhausted() {
    let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    fixture.cache.seed(&stale_key(), keys(&[key("a")]));
    fixture.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(-3_200.5).unwrap()),
    );
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
    let exception = exception_for(&error);
    assert_eq!(exception.code, "503");
    assert_eq!(exception.error_type, "auth_provider_unavailable");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(fixture.http.requests().len(), 3);
}

#[tokio::test]
async fn configured_stale_ttl() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 300.0, 1_200.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    fixture.resolver.get_public_key(Some("a")).await.unwrap();
    let writes = fixture.cache.writes();
    assert!(writes[0].ttl.total_cmp(&300.0_f64).is_eq());
    assert!(writes[1].ttl.total_cmp(&1_500.0_f64).is_eq());
    assert!(writes[2].ttl.total_cmp(&1_500.0_f64).is_eq());

    // the configured window is 300 + 1200: a copy written now is served until exactly 1500 seconds old
    let served = fixture(settings(Some(JWKS_URL), 300.0, 1_200.0), 1_000.0, outage());
    served.cache.seed(&stale_key(), keys(&[key("a")]));
    served.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(1_000.0 - 1_500.0).unwrap()),
    );
    let value = served.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));

    let expired = fixture(settings(Some(JWKS_URL), 300.0, 1_200.0), 1_000.0, outage());
    expired.cache.seed(&stale_key(), keys(&[key("a")]));
    expired.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(1_000.0 - 1_501.0).unwrap()),
    );
    let error = expired
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
}

#[tokio::test]
async fn zero_stale_ttl_fail_closed() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 0.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(fixture.cache.write_keys(), vec![KEYS_KEY.to_owned()]);

    // a seeded copy and timestamp do not make an outage servable at stale_ttl 0
    let closed = fixture(settings(Some(JWKS_URL), 600.0, 0.0), 1_000.0, outage());
    closed.cache.seed(&stale_key(), keys(&[key("a")]));
    closed.cache.seed(
        &timestamp_key(),
        Json::Number(serde_json::Number::from_f64(1_000.0).unwrap()),
    );
    let error = closed.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::Unreachable { .. }));
    assert_eq!(closed.http.requests().len(), 3);
    assert_eq!(closed.cache.write_keys(), vec![unreachable_key()]);
}

#[tokio::test]
async fn outage_backoff_30_seconds() {
    let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    let writes = fixture.cache.writes();
    assert_eq!(writes[0].key, unreachable_key());
    assert!(writes[0].ttl.total_cmp(&30.0_f64).is_eq());

    // the marker is what makes a sustained outage cost one fetch per 30 seconds
    fixture.clock.advance(29.0);
    fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert_eq!(fixture.http.requests().len(), 3);

    // once the marker is gone, the next call fetches again
    fixture.cache.remove(&unreachable_key());
    fixture.http.push(ScriptedHttp::answer(&keys(&[key("a")])));
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert_eq!(fixture.http.requests().len(), 4);
}
