#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::task::JoinSet;

use super::{client, config, upstream};
use crate::agent_endpoints::databricks_oauth::config::DatabricksAppOAuthConfig;
use crate::agent_endpoints::databricks_oauth::token_cache::DatabricksAppOAuthTokenCache;

#[derive(Clone, Copy, Debug)]
enum Between {
    Nothing,
    RotateSecret,
    FlushCache,
}

// Rounds of concurrent callers on one credential, with the clock and the cache changed between rounds.
#[derive(Clone, Copy, Debug)]
struct Scenario {
    expires_in: u32,
    callers: usize,
    rounds: usize,
    advance: u32,
    between: Between,
    delay_ms: u64,
}

const BASE: Scenario = Scenario {
    expires_in: 3600,
    callers: 1,
    rounds: 2,
    advance: 0,
    between: Between::Nothing,
    delay_ms: 0,
};

async fn fetch_concurrently(
    cache: &Arc<DatabricksAppOAuthTokenCache>,
    credentials: &DatabricksAppOAuthConfig,
    callers: usize,
) {
    let mut calls = JoinSet::new();
    for _ in 0..callers {
        let shared_cache = Arc::clone(cache);
        let own_credentials = credentials.clone();
        calls.spawn(async move {
            shared_cache
                .async_get_token(&own_credentials)
                .await
                .unwrap()
        });
    }
    while let Some(token) = calls.join_next().await {
        assert_eq!(token.unwrap().expose_secret(), "tok");
    }
}

async fn run(scenario: Scenario) -> (usize, usize) {
    let body = format!(
        r#"{{"access_token":"tok","expires_in":{}}}"#,
        scenario.expires_in
    );
    let server = upstream(200, &body, Duration::from_millis(scenario.delay_ms)).await;
    let now = Arc::new(AtomicU32::new(0));
    let reader = Arc::clone(&now);
    let cache = Arc::new(
        DatabricksAppOAuthTokenCache::new(
            client(),
            Arc::new(move || f64::from(reader.load(Ordering::SeqCst))),
        )
        .unwrap(),
    );
    let mut secret = "secret".to_owned();
    for round in 0..scenario.rounds {
        if round > 0 {
            now.fetch_add(scenario.advance, Ordering::SeqCst);
            match scenario.between {
                Between::Nothing => {}
                Between::RotateSecret => secret.push('!'),
                Between::FlushCache => cache.flush_cache().await.unwrap(),
            }
        }
        let credentials = config(&server.url, &secret);
        fetch_concurrently(&cache, &credentials, scenario.callers).await;
    }
    let requests = server.requests.lock().unwrap().len();
    (requests, cache.live_lock_count().await)
}

#[rstest]
#[case::cache_hit(BASE, (1, 1))]
#[case::cache_hit_just_before_expiry(Scenario { expires_in: 120, advance: 60, ..BASE }, (1, 1))]
#[case::cache_expired(Scenario { expires_in: 120, advance: 61, ..BASE }, (2, 1))]
#[case::request_coalescing(Scenario { callers: 8, rounds: 1, delay_ms: 300, ..BASE }, (1, 1))]
#[case::short_lived_token_not_cached(Scenario { expires_in: 30, rounds: 3, ..BASE }, (3, 0))]
#[case::secret_rotation_fetches_again(Scenario { between: Between::RotateSecret, ..BASE }, (2, 2))]
#[case::expired_lock_is_dropped_with_its_token(Scenario { expires_in: 120, advance: 61, between: Between::RotateSecret, ..BASE }, (2, 1))]
#[case::flush_cache_clears_tokens_and_locks(Scenario { between: Between::FlushCache, ..BASE }, (2, 1))]
#[tokio::test]
async fn databricks_oauth_tests_behavior(
    #[case] scenario: Scenario,
    #[case] expected: (usize, usize),
) {
    assert_eq!(run(scenario).await, expected);
}
