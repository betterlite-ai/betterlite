#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rstest::rstest;

use super::{
    InMemoryTokenCacheBackend, OAuthToken, SystemWallClock, TokenCacheBackend as _,
    TokenCacheEntries, TokenKey, TokenStoreUnavailable, WallClock, iso_to_epoch,
};

#[derive(Debug)]
struct ManualClock(Mutex<DateTime<Utc>>);

impl ManualClock {
    fn set(&self, now: DateTime<Utc>) {
        *self.0.lock().unwrap() = now;
    }
}

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn token(access_token: &str) -> OAuthToken {
    OAuthToken {
        access_token: access_token.to_owned(),
        expires_at: None,
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: None,
    }
}

fn key(user_id: &str, server_id: &str) -> TokenKey {
    TokenKey {
        user_id: user_id.to_owned(),
        server_id: server_id.to_owned(),
    }
}

fn backend(clock: Arc<ManualClock>, max_size: usize) -> InMemoryTokenCacheBackend {
    InMemoryTokenCacheBackend {
        max_size,
        clock,
        entries: TokenCacheEntries::default(),
    }
}

fn manual_clock(seconds: i64) -> Arc<ManualClock> {
    Arc::new(ManualClock(Mutex::new(at(seconds))))
}

#[test]
fn oauth_token_debug_masks_the_access_and_refresh_tokens() {
    let mut value = token("access-secret");
    value.refresh_token = Some("refresh-secret".to_owned());
    value.scopes = vec!["read".to_owned()];
    let rendered = format!("{value:?}");
    assert!(!rendered.contains("access-secret"));
    assert!(!rendered.contains("refresh-secret"));
    assert!(rendered.contains("***"));
    assert!(rendered.contains("has_refresh_token: true"));
    assert!(rendered.contains("read"));
}

#[test]
fn oauth_token_debug_reports_a_missing_refresh_token() {
    let rendered = format!("{:?}", token("access-secret"));
    assert!(rendered.contains("has_refresh_token: false"));
    assert!(!rendered.contains("access-secret"));
}

#[test]
fn oauth_token_without_an_expiry_is_never_expired() {
    assert!(!token("access").is_expired(at(1_000_000), Duration::from_secs(60)));
}

#[test]
fn oauth_token_is_expired_at_the_skew_boundary() {
    let mut value = token("access");
    value.expires_at = Some(at(1_000));
    assert!(value.is_expired(at(940), Duration::from_secs(60)));
    assert!(!value.is_expired(at(939), Duration::from_secs(60)));
}

#[test]
fn oauth_token_is_expired_past_the_expiry() {
    let mut value = token("access");
    value.expires_at = Some(at(1_000));
    assert!(value.is_expired(at(1_001), Duration::from_secs(60)));
}

#[test]
fn system_wall_clock_reads_the_current_time() {
    let before = Utc::now();
    let now = SystemWallClock.now();
    let after = Utc::now();
    assert!(now >= before);
    assert!(now <= after);
}

#[test]
fn token_store_unavailable_displays_its_detail() {
    let failure = TokenStoreUnavailable {
        detail: "Database not connected".to_owned(),
    };
    assert_eq!(failure.to_string(), "Database not connected");
}

#[rstest]
#[case::offset("2024-01-01T00:00:00+00:00", Some(1_704_067_200))]
#[case::zulu("2024-01-01T00:00:00Z", Some(1_704_067_200))]
#[case::positive_offset("2024-01-01T01:00:00+01:00", Some(1_704_067_200))]
#[case::naive("2024-01-01T00:00:00", Some(1_704_067_200))]
#[case::space_separator("2024-01-01 00:00:00", Some(1_704_067_200))]
#[case::date_only("2024-01-01", Some(1_704_067_200))]
#[case::fraction("2024-01-01T00:00:00.500", Some(1_704_067_200))]
#[case::malformed("not-a-date", None)]
fn iso_to_epoch_reads_the_stored_expiry_forms(#[case] text: &str, #[case] expected: Option<i64>) {
    assert_eq!(
        iso_to_epoch(text).map(|parsed| parsed.timestamp()),
        expected
    );
}

#[tokio::test]
async fn in_memory_cache_backend_returns_a_hit_within_the_ttl() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 4096);
    let token_key = key("user", "server");
    cache
        .set(&token_key, &token("access"), Duration::from_secs(10))
        .await;
    clock.set(at(109));
    assert_eq!(cache.get(&token_key).await, Some(token("access")));
}

#[tokio::test]
async fn in_memory_cache_backend_expires_at_the_ttl_boundary() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 4096);
    let token_key = key("user", "server");
    cache
        .set(&token_key, &token("access"), Duration::from_secs(10))
        .await;
    clock.set(at(110));
    assert_eq!(cache.get(&token_key).await, None);
    assert!(cache.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn in_memory_cache_backend_returns_a_miss_for_an_unknown_key() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 4096);
    assert_eq!(cache.get(&key("user", "server")).await, None);
}

#[tokio::test]
async fn in_memory_cache_backend_evicts_the_oldest_entry_on_overflow() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 2);
    let first = key("user", "first");
    let second = key("user", "second");
    let third = key("user", "third");
    cache
        .set(&first, &token("first"), Duration::from_secs(60))
        .await;
    cache
        .set(&second, &token("second"), Duration::from_secs(60))
        .await;
    assert_eq!(cache.get(&first).await, Some(token("first")));
    cache
        .set(&third, &token("third"), Duration::from_secs(60))
        .await;
    assert_eq!(cache.get(&first).await, None);
    assert_eq!(cache.get(&second).await, Some(token("second")));
    assert_eq!(cache.get(&third).await, Some(token("third")));
}

#[tokio::test]
async fn in_memory_cache_backend_with_zero_size_keeps_only_the_newest_entry() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 0);
    let first = key("user", "first");
    let second = key("user", "second");
    cache
        .set(&first, &token("first"), Duration::from_secs(60))
        .await;
    assert_eq!(cache.get(&first).await, Some(token("first")));
    cache
        .set(&second, &token("second"), Duration::from_secs(60))
        .await;
    assert_eq!(cache.get(&first).await, None);
    assert_eq!(cache.get(&second).await, Some(token("second")));
}

#[tokio::test]
async fn in_memory_cache_backend_delete_removes_an_entry() {
    let clock = manual_clock(100);
    let cache = backend(Arc::clone(&clock), 4096);
    let token_key = key("user", "server");
    cache
        .set(&token_key, &token("access"), Duration::from_secs(60))
        .await;
    cache.delete(&token_key).await;
    assert_eq!(cache.get(&token_key).await, None);
}
