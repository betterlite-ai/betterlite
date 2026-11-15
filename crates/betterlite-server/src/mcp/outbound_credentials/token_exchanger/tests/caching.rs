use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::super::super::refresh_store::InProcessRefreshCoordinator;
use super::super::super::{InMemoryTokenCacheBackend, TokenCacheEntries, TokenKey, WallClock};
use super::super::{OboTokenExchanger, cache_key};
use super::support::{
    ManualClock, RecordingPost, Scripted, TestExchanger, body, clock, config, exchanger, server,
    server_named,
};

#[tokio::test]
async fn a_cached_token_skips_the_second_post() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"x","expires_in":3600}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let first = exchanger
        .exchange("caller-jwt", &server(), &config(), "")
        .await
        .unwrap();
    let second = exchanger
        .exchange("caller-jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(exchanger.post.calls().len(), 1);
}

#[tokio::test]
async fn a_rotated_subject_token_reexchanges() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    let first = exchanger
        .exchange("jwt-1", &server(), &config(), "")
        .await
        .unwrap();
    let second = exchanger
        .exchange("jwt-2", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(first.access_token, "first");
    assert_eq!(second.access_token, "second");
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn a_rotated_config_reexchanges_before_the_ttl() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    let mut rotated = config();
    rotated.audience = Some("https://new.example.com".to_owned());
    rotated.scopes = vec!["s3".to_owned()];
    exchanger
        .exchange("jwt", &server(), &rotated, "")
        .await
        .unwrap();
    let calls = exchanger.post.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].form.get("audience"),
        Some(&"https://new.example.com".to_owned())
    );
    assert_eq!(calls[1].form.get("scope"), Some(&"s3".to_owned()));
}

#[tokio::test]
async fn different_tenants_do_not_share_a_cache_entry() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    exchanger
        .exchange("jwt", &server(), &config(), "globex")
        .await
        .unwrap();
    exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn different_servers_do_not_share_a_cache_entry() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server_named("first"), &config(), "")
        .await
        .unwrap();
    exchanger
        .exchange("jwt", &server_named("second"), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn invalidate_forces_a_fresh_exchange() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    exchanger
        .invalidate("jwt", &server(), &config(), "acme")
        .await;
    let second = exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    assert_eq!(second.access_token, "second");
    assert_eq!(exchanger.post.calls().len(), 2);
}

#[tokio::test]
async fn invalidate_targets_only_the_matching_tenant() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"second","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"third","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    exchanger
        .exchange("jwt", &server(), &config(), "globex")
        .await
        .unwrap();
    exchanger
        .invalidate("jwt", &server(), &config(), "acme")
        .await;
    let globex = exchanger
        .exchange("jwt", &server(), &config(), "globex")
        .await
        .unwrap();
    let acme = exchanger
        .exchange("jwt", &server(), &config(), "acme")
        .await
        .unwrap();
    assert_eq!(globex.access_token, "second");
    assert_eq!(acme.access_token, "third");
    assert_eq!(exchanger.post.calls().len(), 3);
}

#[tokio::test]
async fn concurrent_callers_share_one_exchange() {
    let post = RecordingPost::new(vec![Scripted::Body(body(
        r#"{"access_token":"shared","expires_in":3600}"#,
    ))]);
    let exchanger = exchanger(post, clock(1000));
    let server = server();
    let config = config();
    let first = exchanger.exchange("jwt", &server, &config, "");
    let second = exchanger.exchange("jwt", &server, &config, "");
    let third = exchanger.exchange("jwt", &server, &config, "");
    let (first_result, second_result, third_result) = tokio::join!(first, second, third);
    assert_eq!(first_result.unwrap().access_token, "shared");
    assert_eq!(second_result.unwrap().access_token, "shared");
    assert_eq!(third_result.unwrap().access_token, "shared");
    assert_eq!(exchanger.post.calls().len(), 1);
}

#[tokio::test]
async fn a_reentrant_refresh_reuses_the_cached_token() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"cached","expires_in":3600}"#)),
        Scripted::Body(body(r#"{"access_token":"unused","expires_in":3600}"#)),
    ]);
    let exchanger = exchanger(post, clock(1000));
    let config = config();
    let key = TokenKey {
        user_id: cache_key::cache_key("jwt", "", &config),
        server_id: "srv".to_owned(),
    };
    let form = BTreeMap::new();
    let headers = BTreeMap::new();
    let first = exchanger
        .refresh(&key, "https://idp/token", &form, &headers)
        .await
        .unwrap();
    let second = exchanger
        .refresh(&key, "https://idp/token", &form, &headers)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(exchanger.post.calls().len(), 1);
}

#[tokio::test]
async fn custom_tunables_change_the_default_ttl() {
    let post = RecordingPost::new(vec![
        Scripted::Body(body(r#"{"access_token":"first"}"#)),
        Scripted::Body(body(r#"{"access_token":"second"}"#)),
    ]);
    let clock = clock(1000);
    let observed: Arc<ManualClock> = Arc::clone(&clock);
    let manual_clock: Arc<ManualClock> = Arc::clone(&clock);
    let cache_clock: Arc<dyn WallClock> = manual_clock;
    let exchanger: TestExchanger = OboTokenExchanger {
        post,
        cache: InMemoryTokenCacheBackend {
            max_size: 16,
            clock: cache_clock,
            entries: TokenCacheEntries::default(),
        },
        coordinator: InProcessRefreshCoordinator::default(),
        clock,
        default_ttl: Duration::from_secs(300),
        min_ttl: Duration::from_secs(5),
        expiry_buffer: Duration::from_secs(10),
    };
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    observed.set(1299);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 1);
    observed.set(1300);
    exchanger
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(exchanger.post.calls().len(), 2);
}
